//! Reading the reports that a type describes from its attributes.

use syn::{
    Attribute, Data, DeriveInput, Error, Field, Fields, Ident, Meta, PathArguments, Result, Type,
    spanned::Spanned,
};

use super::{DiagnosticField, DiagnosticVariant, FieldPart, FieldShape, Note, NoteKind};

/// The reports that `input` describes: one for a struct, or one per variant
/// of an enum.
pub(super) fn variants(input: &DeriveInput) -> Result<Vec<DiagnosticVariant>> {
    match &input.data {
        Data::Struct(data) => {
            Ok(vec![DiagnosticVariant::parse(&input.ident, false, &input.attrs, &data.fields)?])
        }
        Data::Enum(data) => {
            if let Some(attr) = input.attrs.iter().find(|attr| is_message(attr)) {
                return Err(Error::new_spanned(attr, "the messages of an enum go on each variant"));
            }
            if let Some(attr) = input.attrs.iter().find(|attr| is_field_only(attr)) {
                return Err(Error::new_spanned(attr, "this attribute goes on a field"));
            }
            if data.variants.is_empty() {
                return Err(Error::new(input.ident.span(), "a diagnostic enum needs a variant"));
            }
            data.variants
                .iter()
                .map(|item| DiagnosticVariant::parse(&item.ident, true, &item.attrs, &item.fields))
                .collect()
        }
        Data::Union(_) => Err(Error::new(input.ident.span(), "a diagnostic cannot be a union")),
    }
}

impl DiagnosticVariant {
    /// The report of the struct or variant `ident`, from its `attrs` and
    /// `fields`.
    fn parse(
        ident: &Ident,
        is_variant: bool,
        attrs: &[Attribute],
        fields: &Fields,
    ) -> Result<Self> {
        let mut message = None;
        let mut notes = Vec::new();
        for attr in attrs {
            if attr.path().is_ident("diag") {
                if message.is_some() {
                    return Err(Error::new_spanned(attr, "a diagnostic has one `#[diag]`"));
                }
                message = Some(attr.parse_args::<Ident>()?);
            } else if let Some(kind) = NoteKind::of(attr) {
                notes.push(Note::parse(kind, attr)?);
            } else if is_field_only(attr) {
                return Err(Error::new_spanned(attr, "this attribute goes on a field"));
            }
        }
        let Some(message) = message else {
            let error = "a diagnostic needs its message, `#[diag(message_id)]`";
            return Err(Error::new(ident.span(), error));
        };

        let fields = match fields {
            Fields::Named(fields) => {
                fields.named.iter().map(DiagnosticField::parse).collect::<Result<_>>()?
            }
            Fields::Unit => Vec::new(),
            Fields::Unnamed(fields) => {
                let error =
                    "the fields of a diagnostic must be named, for its messages to refer to";
                return Err(Error::new_spanned(fields, error));
            }
        };
        Ok(Self { ident: ident.clone(), is_variant, message, notes, fields })
    }
}

impl DiagnosticField {
    /// A named field, and what its attributes add to the report.
    fn parse(field: &Field) -> Result<Self> {
        let ident = field.ident.clone().expect("the fields are named");
        let shape = FieldShape::of(&field.ty);
        let mut parts = Vec::new();
        for attr in &field.attrs {
            let path = attr.path();
            let part = if path.is_ident("primary_span") {
                attr.meta.require_path_only()?;
                FieldPart::Span(None)
            } else if path.is_ident("label") {
                FieldPart::Span(Some(Note::parse(NoteKind::Label, attr)?))
            } else if path.is_ident("subdiagnostic") {
                attr.meta.require_path_only()?;
                FieldPart::Subdiagnostic
            } else if let Some(kind) = NoteKind::of(attr) {
                FieldPart::Note(Note::parse(kind, attr)?)
            } else if path.is_ident("diag") {
                return Err(Error::new_spanned(attr, "`#[diag]` goes on a struct or a variant"));
            } else {
                continue;
            };
            if shape == FieldShape::Bool && !matches!(part, FieldPart::Note(_)) {
                return Err(Error::new_spanned(attr, "a `bool` field can only add a note"));
            }
            parts.push(part);
        }
        Ok(Self { ident, shape, parts })
    }
}

impl Note {
    /// The note of `kind` that `attr` adds: the attribute of the report's
    /// message named after the kind, as `#[help]` is `.help`, or the one
    /// that it names, as `#[help(rename)]` is `.rename`.
    fn parse(kind: NoteKind, attr: &Attribute) -> Result<Self> {
        let attribute = match &attr.meta {
            Meta::Path(path) => Ident::new(kind.attribute(), path.span()),
            _ => attr.parse_args()?,
        };
        Ok(Self { kind, attribute })
    }
}

impl FieldShape {
    /// The shape of a field of type `ty`, from how it is written: the type
    /// is not resolved.
    fn of(ty: &Type) -> Self {
        let Type::Path(ty) = ty else {
            return Self::Other;
        };
        if ty.path.is_ident("bool") {
            return Self::Bool;
        }
        match ty.path.segments.last() {
            Some(last)
                if last.ident == "Option"
                    && matches!(last.arguments, PathArguments::AngleBracketed(_)) =>
            {
                Self::Option
            }
            _ => Self::Other,
        }
    }
}

impl NoteKind {
    /// The kind of note that `attr` adds, if it is `#[note]` or `#[help]`.
    fn of(attr: &Attribute) -> Option<Self> {
        let path = attr.path();
        if path.is_ident("note") {
            Some(Self::Note)
        } else if path.is_ident("help") {
            Some(Self::Help)
        } else {
            None
        }
    }
}

/// Whether `attr` names a message of the report: its title or a note.
fn is_message(attr: &Attribute) -> bool {
    attr.path().is_ident("diag") || NoteKind::of(attr).is_some()
}

/// Whether `attr` is one that only goes on a field.
fn is_field_only(attr: &Attribute) -> bool {
    let path = attr.path();
    path.is_ident("primary_span") || path.is_ident("label") || path.is_ident("subdiagnostic")
}
