//! A stand-in for [`unicode_names2`](https://crates.io/crates/unicode_names2),
//! which Ruff's parser uses only to decode the `\N{…}` escapes of string
//! literals. Its table of character names is most of a megabyte, and zenith
//! never reads what a string holds, but for the module that
//! `importlib.import_module` imports, which never has such an escape.

/// The character named `name`: always U+FFFD, so that every escape decodes,
/// and a string with one is not a syntax error.
pub fn character(_name: &str) -> Option<char> {
    Some(char::REPLACEMENT_CHARACTER)
}
