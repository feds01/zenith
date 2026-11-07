//! What is wrong in a syntax error of a Haskell file, and what the grammar
//! expected where it found something else.

use std::fmt;

use zenith_diagnostics::message;

use super::lexer::LexError;

/// What is wrong in a syntax error of a Haskell file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HaskellSyntaxError {
    /// The source is 4 GiB or longer, which is more than offsets can address.
    SourceTooLarge,

    /// A `{-` comment has no `-}` to end it.
    UnterminatedComment,

    /// A string has no `"` to end it on its line.
    UnterminatedString,

    /// A character that is no part of any lexeme, such as a control
    /// character.
    InvalidCharacter,

    /// The grammar expected something else where the error is.
    Expected(Expected),

    /// A lexeme that the grammar has no place for, such as one after the end
    /// of an import, as written.
    Unexpected(String),

    /// A declaration ends before the grammar has read all of it.
    UnexpectedEnd,
}

/// What the grammar expected where it found something else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Expected {
    /// The name of a module, such as `Data.Map`.
    ModuleName,

    /// The name of a package, in quotes, as a package-qualified import names
    /// one: words of letters and digits, apart by `-`.
    PackageName,

    /// A name, or an operator in parentheses, as an import or export list
    /// holds.
    Name,

    /// A lexeme that must be written as it is, such as `where` or `)`.
    Token(&'static str),
}

impl fmt::Display for HaskellSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceTooLarge => f.write_str(&message!(haskell_source_too_large)),
            Self::UnterminatedComment => f.write_str(&message!(haskell_unterminated_comment)),
            Self::UnterminatedString => f.write_str(&message!(haskell_unterminated_string)),
            Self::InvalidCharacter => f.write_str(&message!(haskell_invalid_character)),
            Self::Expected(expected) => expected.fmt(f),
            Self::Unexpected(token) => f.write_str(&message!(haskell_unexpected_token, token)),
            Self::UnexpectedEnd => f.write_str(&message!(haskell_unexpected_end)),
        }
    }
}

impl fmt::Display for Expected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModuleName => f.write_str(&message!(haskell_expected_module_name)),
            Self::PackageName => f.write_str(&message!(haskell_expected_package_name)),
            Self::Name => f.write_str(&message!(haskell_expected_name)),
            Self::Token(token) => f.write_str(&message!(haskell_expected_token, token)),
        }
    }
}

impl From<LexError> for HaskellSyntaxError {
    fn from(error: LexError) -> Self {
        match error {
            LexError::UnterminatedComment => Self::UnterminatedComment,
            LexError::UnterminatedString => Self::UnterminatedString,
            LexError::Character => Self::InvalidCharacter,
        }
    }
}
