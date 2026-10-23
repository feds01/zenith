//! The `message!` macro, which formats a message of the catalogue, and the
//! code that formats one, which the derive of `Diagnostic` shares.

use std::collections::BTreeSet;

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{
    Error, Expr, Ident, Result, Token,
    ext::IdentExt,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

use crate::catalogue::{Catalogue, Variables};

/// A message of the catalogue, or an attribute of one, as code names it:
/// `config_unknown_key`, or `config_no_languages.help`.
pub(crate) struct MessageName {
    /// The id of the message.
    pub(crate) id: Ident,

    /// The attribute, if it is one.
    pub(crate) attribute: Option<Ident>,
}

impl MessageName {
    /// The variables of the message, which must be in the catalogue.
    pub(crate) fn variables(&self) -> Result<Variables> {
        let catalogue = Catalogue::get().map_err(|error| Error::new(self.span(), error))?;
        let attribute = self.attribute.as_ref().map(Ident::to_string);
        catalogue
            .variables(&self.id.to_string(), attribute.as_deref())
            .map_err(|error| Error::new(self.span(), error))
    }

    /// Where the name is written: its attribute, if it has one.
    pub(crate) fn span(&self) -> Span {
        self.attribute.as_ref().map_or(self.id.span(), Ident::span)
    }

    /// The expression that formats the message, with its `variables` each
    /// set to the expression that `value` gives for the name.
    pub(crate) fn format(
        &self,
        variables: &Variables,
        value: impl Fn(&str) -> TokenStream,
    ) -> TokenStream {
        let id = self.id.to_string();
        let attribute = match &self.attribute {
            Some(attribute) => {
                let attribute = attribute.to_string();
                quote! { ::std::option::Option::Some(#attribute) }
            }
            None => quote! { ::std::option::Option::None },
        };
        if variables.names.is_empty() {
            return quote! {
                ::zenith_diagnostics::__private::message(
                    #id,
                    #attribute,
                    &::zenith_diagnostics::__private::MessageArgs::new(),
                )
            };
        }

        // The arguments are named hygienically, so that no value refers to
        // them by mistake.
        let args = Ident::new("args", Span::mixed_site());
        let sets = variables.names.iter().map(|(name, number)| {
            let value = value(name);
            if *number {
                quote! { #args.number(#name, #value); }
            } else {
                quote! { #args.text(#name, &(#value)); }
            }
        });
        quote! {{
            let mut #args = ::zenith_diagnostics::__private::MessageArgs::new();
            #(#sets)*
            ::zenith_diagnostics::__private::message(#id, #attribute, &#args)
        }}
    }
}

impl Parse for MessageName {
    fn parse(input: ParseStream) -> Result<Self> {
        let id = input.parse()?;
        let attribute =
            if input.parse::<Option<Token![.]>>()?.is_some() { Some(input.parse()?) } else { None };
        Ok(Self { id, attribute })
    }
}

/// The input of `message!`: a message, then its variables, each `name =
/// value`, or `name` for a binding of that name.
pub(crate) struct MessageCall {
    /// The message.
    name: MessageName,

    /// The variables, in order.
    arguments: Vec<(Ident, Expr)>,
}

impl Parse for MessageCall {
    fn parse(input: ParseStream) -> Result<Self> {
        let name = input.parse()?;
        let mut arguments = Vec::new();
        if input.parse::<Option<Token![,]>>()?.is_some() {
            let parsed = Punctuated::<Argument, Token![,]>::parse_terminated(input)?;
            arguments.extend(parsed.into_iter().map(|argument| (argument.name, argument.value)));
        }
        Ok(Self { name, arguments })
    }
}

/// A variable of `message!`: `name = value`, or `name`.
struct Argument {
    /// The name of the variable.
    name: Ident,

    /// Its value.
    value: Expr,
}

impl Parse for Argument {
    fn parse(input: ParseStream) -> Result<Self> {
        let name: Ident = input.parse()?;
        let value = if input.parse::<Option<Token![=]>>()?.is_some() {
            input.parse()?
        } else {
            syn::parse_quote! { #name }
        };
        Ok(Self { name, value })
    }
}

impl MessageCall {
    /// The expression that formats the message, or why its variables are
    /// not the message's.
    pub(crate) fn expand(&self) -> Result<TokenStream> {
        let variables = self.name.variables()?;
        let mut given = BTreeSet::new();
        for (name, _) in &self.arguments {
            let key = name.unraw().to_string();
            if !variables.names.contains_key(&key) {
                let message = format!("the message has no variable `${key}`");
                return Err(Error::new(name.span(), message));
            }
            if !given.insert(key) {
                return Err(Error::new(name.span(), "this variable is given twice"));
            }
        }
        if let Some(missing) = variables.names.keys().find(|name| !given.contains(*name)) {
            let message = format!("the message needs the variable `${missing}`");
            return Err(Error::new(self.name.span(), message));
        }

        Ok(self.name.format(&variables, |name| {
            let (_, value) = self
                .arguments
                .iter()
                .find(|(given, _)| given.unraw() == name)
                .expect("every variable is given");
            quote! { #value }
        }))
    }
}
