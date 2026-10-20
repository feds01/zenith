//! The comments of a source file, as the suppression logic needs them.

use zenith_utils::location::ByteRange;

/// Whether a [`Comment`] runs to the end of its line, or is delimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentKind {
    /// A comment that runs to the end of its line, e.g. `// …`.
    Line,

    /// A comment between delimiters, which may span lines, e.g. `/* … */`.
    Block,
}

/// A comment in a source file, as a language's parser reports it. Only what
/// finding [`Suppression`](crate::Suppression)s needs is here, so every
/// language can fill it in from its own syntax tree or lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Comment {
    /// The comment, with its delimiters.
    pub range: ByteRange,

    /// The text of the comment, without its delimiters.
    pub content: ByteRange,

    /// Whether it is a line or a block comment.
    pub kind: CommentKind,

    /// Whether code comes before the comment on its first line.
    pub trailing: bool,

    /// Whether a line break follows the comment, with nothing but whitespace
    /// between them.
    pub followed_by_newline: bool,

    /// The byte offset where the token after the comment starts. Other
    /// comments are not tokens, so it skips them.
    pub attached_to: u32,
}
