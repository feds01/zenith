//! The `Diagnostic` and `Subdiagnostic` impls that make the reports a type
//! describes, from the messages of the catalogue.

use std::collections::BTreeSet;

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{DeriveInput, Error, Ident, Result, ext::IdentExt};

use super::{DiagnosticField, DiagnosticVariant, FieldPart, FieldShape, Note, NoteKind};
use crate::{catalogue::Catalogue, message::MessageName};

/// The impls of `input`, whose reports are `variants`, or why the messages
/// that they name are not what the fields can give.
pub(super) fn impl_diagnostic(
    input: &DeriveInput,
    variants: &[DiagnosticVariant],
) -> Result<TokenStream> {
    let ident = &input.ident;
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    // The report that the contents are added to. Its name is hygienic, so
    // that no field shadows it.
    let report = Ident::new("report", Span::mixed_site());

    let mut titles = Vec::new();
    let mut contents = Vec::new();
    for variant in variants {
        let pattern = variant.pattern();
        let title = variant.format(None, None)?;
        titles.push(quote! { #pattern => #title, });
        let statements = variant.statements(&report)?;
        contents.push(quote! { #pattern => { #(#statements)* } });
        variant.check_attributes()?;
    }

    // Every field is bound, whether or not a message shows it.
    Ok(quote! {
        impl #impl_generics ::zenith_diagnostics::Diagnostic for #ident #type_generics #where_clause {
            #[allow(unused_variables)]
            fn title(&self) -> ::std::string::String {
                match self { #(#titles)* }
            }
        }

        impl #impl_generics ::zenith_diagnostics::Subdiagnostic for #ident #type_generics #where_clause {
            #[allow(unused_variables)]
            fn add_to(&self, #report: &mut ::zenith_diagnostics::Report) {
                match self { #(#contents)* }
            }
        }
    })
}

impl DiagnosticVariant {
    /// The pattern that matches the variant and binds each of its fields by
    /// name. A braced pattern matches unit structs and variants too.
    fn pattern(&self) -> TokenStream {
        let fields = self.fields.iter().map(|field| &field.ident);
        let ident = &self.ident;
        if self.is_variant {
            quote! { Self::#ident { #(#fields),* } }
        } else {
            quote! { Self { #(#fields),* } }
        }
    }

    /// The statements that add the excerpts and notes of the variant to
    /// `report`: the excerpts, then the notes on the variant, then those of
    /// its fields.
    fn statements(&self, report: &Ident) -> Result<Vec<TokenStream>> {
        let mut statements = Vec::new();
        for field in &self.fields {
            for part in &field.parts {
                let FieldPart::Span(label) = part else {
                    continue;
                };
                let ident = &field.ident;
                let add = match label {
                    Some(label) => {
                        let label = self.format(Some(label), Some(field))?;
                        quote! { #report.add_labelled_span(*#ident, #label); }
                    }
                    None => quote! { #report.add_span(*#ident); },
                };
                statements.push(field.when_present(add));
            }
        }

        for note in &self.notes {
            statements.push(note.add(self.format(Some(note), None)?, report));
        }

        for field in &self.fields {
            for part in &field.parts {
                let ident = &field.ident;
                let add = match part {
                    FieldPart::Span(_) => continue,
                    FieldPart::Note(note) => {
                        note.add(self.format(Some(note), Some(field))?, report)
                    }
                    FieldPart::Subdiagnostic => {
                        quote! { ::zenith_diagnostics::Subdiagnostic::add_to(#ident, #report); }
                    }
                };
                statements.push(field.when_present(add));
            }
        }
        Ok(statements)
    }

    /// The expression that formats the variant's message, or the attribute
    /// of it that `note` is, with its variables set to the fields of the same
    /// names. `own` is the field that the note is on, if any: an `Option`
    /// field is only shown by its own notes, in which it is the value inside.
    fn format(&self, note: Option<&Note>, own: Option<&DiagnosticField>) -> Result<TokenStream> {
        let message = MessageName {
            id: self.message.clone(),
            attribute: note.map(|note| note.attribute.clone()),
        };
        let variables = message.variables()?;
        for name in variables.names.keys() {
            let Some(field) = self.field(name) else {
                let error =
                    format!("the message shows `${name}`, which `{}` has no field for", self.ident);
                return Err(Error::new(message.span(), error));
            };
            let is_own = own.is_some_and(|own| own.ident == field.ident);
            if field.shape == FieldShape::Option && !is_own {
                let error = format!(
                    "the message shows `${name}`, an `Option`, which only the notes and labels on \
                     its field can show"
                );
                return Err(Error::new(message.span(), error));
            }
        }

        Ok(message.format(&variables, |name| {
            let ident = &self.field(name).expect("a field").ident;
            if variables.names[name] {
                quote! { *#ident }
            } else {
                quote! { #ident }
            }
        }))
    }

    /// The field that the variable `name` shows, if there is one.
    fn field(&self, name: &str) -> Option<&DiagnosticField> {
        self.fields.iter().find(|field| field.ident.unraw() == name)
    }

    /// Check that the variant shows each attribute of its message, once.
    fn check_attributes(&self) -> Result<()> {
        let notes =
            self.fields.iter().flat_map(|field| &field.parts).filter_map(|part| match part {
                FieldPart::Span(label) => label.as_ref(),
                FieldPart::Note(note) => Some(note),
                FieldPart::Subdiagnostic => None,
            });
        let mut shown = BTreeSet::new();
        for note in self.notes.iter().chain(notes) {
            if !shown.insert(note.attribute.to_string()) {
                let error = "another note or label shows this attribute; name another, e.g. \
                             `#[help(other)]`";
                return Err(Error::new(note.attribute.span(), error));
            }
        }

        let catalogue = Catalogue::get().map_err(|error| Error::new(self.message.span(), error))?;
        let id = self.message.to_string();
        let attributes = catalogue.attributes(&id);
        if let Some(unshown) = attributes.into_iter().find(|name| !shown.contains(*name)) {
            let fix = match unshown {
                "note" | "help" | "label" => format!("`#[{unshown}]`"),
                _ => format!("`#[note({unshown})]`, `#[help({unshown})]` or `#[label({unshown})]`"),
            };
            let error = format!(
                "the message has `.{unshown}`, which `{}` doesn't show: add {fix}",
                self.ident
            );
            return Err(Error::new(self.message.span(), error));
        }
        Ok(())
    }
}

impl DiagnosticField {
    /// `statement`, run only when the field is `Some` or `true`. In the
    /// statement, an `Option` field is bound to the value inside.
    fn when_present(&self, statement: TokenStream) -> TokenStream {
        let ident = &self.ident;
        match self.shape {
            FieldShape::Option => quote! {
                if let ::std::option::Option::Some(#ident) = #ident { #statement }
            },
            FieldShape::Bool => quote! { if *#ident { #statement } },
            FieldShape::Other => statement,
        }
    }
}

impl Note {
    /// The statement that adds `message`, the text of the note, to `report`.
    fn add(&self, message: TokenStream, report: &Ident) -> TokenStream {
        match self.kind {
            NoteKind::Note | NoteKind::Label => quote! { #report.add_note(#message); },
            NoteKind::Help => quote! { #report.add_help(#message); },
        }
    }
}
