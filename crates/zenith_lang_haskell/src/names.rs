//! What Haskell's names are made of, as chapter 2 of the Report gives them,
//! which the lexemes of a file and the paths of the files of its modules
//! both follow.

/// Whether `c` starts a name that is a constructor's: an uppercase or
/// titlecase letter, which has a lowercase form of its own.
///
/// `large → ascLarge | uniLarge`
pub(crate) fn is_large(c: char) -> bool {
    c.is_uppercase() || (c.is_alphabetic() && !c.is_lowercase() && !c.to_lowercase().eq([c]))
}

/// Whether `c` may follow the first character of a name.
pub(crate) fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

/// Whether `name` is a constructor's name, as each part of a module's name
/// is.
///
/// `conid → large {small | large | digit | ' }`
pub(crate) fn is_conid(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(is_large) && chars.all(is_ident)
}
