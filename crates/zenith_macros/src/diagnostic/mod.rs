//! The derive of `Diagnostic`: the reports that a type describes, read from
//! its attributes, and the impls that make them, from the messages of the
//! catalogue that the attributes name.

use proc_macro2::TokenStream;
use syn::{DeriveInput, Ident};

mod emit;
mod parse;

/// The `Diagnostic` and `Subdiagnostic` impls of `input`, or the error in its
/// attributes or in the messages that they name.
pub(crate) fn derive(input: &DeriveInput) -> syn::Result<TokenStream> {
    let variants = parse::variants(input)?;
    emit::impl_diagnostic(input, &variants)
}

/// A struct, or a variant of an enum, and the report that it describes.
struct DiagnosticVariant {
    /// The name of the variant, or of the struct.
    ident: Ident,

    /// Whether it is a variant of an enum.
    is_variant: bool,

    /// The message whose value is the title of the report, from
    /// `#[diag(...)]`, and whose attributes the other messages are.
    message: Ident,

    /// The notes on the struct or variant, in order.
    notes: Vec<Note>,

    /// The fields, in order.
    fields: Vec<DiagnosticField>,
}

/// A note, from `#[note]` or `#[help]`, or a label, from `#[label]`: an
/// attribute of the report's message.
struct Note {
    /// Which of them it is.
    kind: NoteKind,

    /// The name of the attribute: the kind's, as in `.help`, unless the
    /// attribute names another, as in `#[help(rename)]`.
    attribute: Ident,
}

/// The kind of a [`Note`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum NoteKind {
    /// More context about the problem.
    Note,

    /// How to fix the problem.
    Help,

    /// The label of an excerpt.
    Label,
}

impl NoteKind {
    /// The name of the attribute of the report's message that a note of
    /// this kind is, unless it names another.
    fn attribute(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Help => "help",
            Self::Label => "label",
        }
    }
}

/// A field, and what it adds to the report.
struct DiagnosticField {
    /// The name of the field.
    ident: Ident,

    /// Its type, as far as it decides when the field adds to the report.
    shape: FieldShape,

    /// What it adds, in the order of its attributes.
    parts: Vec<FieldPart>,
}

/// The type of a field, as far as it decides when the field adds to the
/// report.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldShape {
    /// An `Option`, which adds only when it is `Some`.
    Option,

    /// A `bool`, which adds only when it is `true`.
    Bool,

    /// Any other type, which always adds.
    Other,
}

/// What a field adds to the report.
enum FieldPart {
    /// An excerpt that marks the field, a span, with a label, if any:
    /// `#[primary_span]` or `#[label]`.
    Span(Option<Note>),

    /// A note.
    Note(Note),

    /// What the field, a `Subdiagnostic`, adds: `#[subdiagnostic]`.
    Subdiagnostic,
}
