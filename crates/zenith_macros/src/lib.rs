//! The macros through which zenith's code names the messages in its
//! catalogue, the Fluent files in `zenith_diagnostics/messages`: the derive
//! of `Diagnostic`, which makes a report from an error type whose messages
//! its attributes name, as rustc's struct diagnostics did, and `message!`,
//! which formats one message. Both check, when the code compiles, that the
//! messages exist and that the code gives each the variables it shows.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

mod catalogue;
mod diagnostic;
mod message;

/// Derive `zenith_diagnostics::Diagnostic`, and the `Subdiagnostic` that
/// adds its excerpts and notes, for a struct, or an enum whose variants are
/// each a diagnostic, from the messages of the catalogue that its
/// attributes name.
///
/// ```ignore
/// #[derive(Diagnostic)]
/// enum ConfigErrorKind {
///     #[diag(config_unknown_key)]
///     UnknownKey { key: String },
///
///     #[diag(config_no_languages)]
///     NoLanguages {
///         #[help]
///         example: Option<String>,
///     },
/// }
/// ```
///
/// with, in the catalogue:
///
/// ```ftl
/// config_unknown_key = unknown key `{$key}`
/// config_no_languages = no language is enabled
///     .help = add a table for one, such as `[languages.{$example}]`
/// ```
///
/// A variable of a message, such as `{$key}`, is the field of that name,
/// shown with `Display`, or given as a number when the message chooses a
/// plural by it. The fields must be named, or there must be none.
///
/// On the struct, or on each variant of an enum:
/// - `#[diag(message_id)]` names the message whose value is the title of the
///   report. The other attributes are attributes of that message, and the
///   struct or variant must show each one. Without it, a field marked `#[diag]`
///   gives the title, and there are no notes or labels.
/// - `#[note]` and `#[help]` add a note, or a note on how to fix the problem:
///   the message's `.note` or `.help`, or the attribute that they name, as in
///   `#[help(rename)]`.
///
/// On a field:
/// - `#[primary_span]`, on a `Span` or a `ByteRange`, or an `Option` of one,
///   adds an excerpt that marks it, and `#[label]` one that labels it with
///   `.label`, or the attribute that it names. A `ByteRange` is in the source
///   that the report is made in, with `Diagnostic::report_in`, and is left out
///   without one.
/// - `#[note]` and `#[help]` add a note. On an `Option`, the note is only added
///   when it is `Some`, and the variable of the field is the value inside; no
///   other message can show the field. On a `bool`, it is only added when it is
///   `true`.
/// - `#[subdiagnostic]` lets the field, a `Subdiagnostic`, or an `Option` of
///   one, add to the report.
/// - `#[diag]`, in place of `#[diag(message_id)]` on the struct or variant,
///   makes the title of the field, a `Diagnostic`, the report's, and lets it
///   add to the report as `#[subdiagnostic]` does.
///
/// Whether a field is an `Option` or a `bool` is read from how its type is
/// written, since a derive can't resolve types: behind an alias, it is
/// neither.
///
/// The excerpts come first, in the order of their fields, then the notes on
/// the struct or variant, then those of its fields, in order.
#[proc_macro_derive(Diagnostic, attributes(diag, note, help, label, primary_span, subdiagnostic))]
pub fn derive_diagnostic(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    diagnostic::derive(&input).unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Format a message of the catalogue, or an attribute of one, to a
/// `String`, with its variables: `message!(config_unknown_key, key = name)`,
/// or `message!(config_unknown_key, key)` for a binding named like the
/// variable, and `message!(config_no_languages.help, example)` for an
/// attribute. Each variable is shown with `Display`, or given as a number
/// when the message chooses a plural by it. The message must exist and be
/// given exactly its variables.
#[proc_macro]
pub fn message(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let call = parse_macro_input!(input as message::MessageCall);
    call.expand().unwrap_or_else(syn::Error::into_compile_error).into()
}

/// The files of the catalogue, as a `&[(&str, &str)]` of each file's name
/// and its contents, for zenith_diagnostics to format the messages from.
#[proc_macro]
pub fn catalogue_sources(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    if !input.is_empty() {
        let error = syn::Error::new(proc_macro2::Span::call_site(), "this takes no input");
        return error.into_compile_error().into();
    }
    let sources = catalogue::sources().map(|(file, contents)| quote! { (#file, #contents) });
    let sources: TokenStream = quote! { &[#(#sources),*] };
    sources.into()
}
