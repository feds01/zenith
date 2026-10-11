//! Errors in configuration files, with their location so that they can be
//! shown in context.

use std::{fmt, io, ops::Range, path::PathBuf};

use zenith_utils::{glob::GlobError, location::ByteRange};

use crate::{
    layer::LayerError,
    selector::{IgnoreImportError, SelectorError},
};

/// An error in a configuration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    /// What is wrong.
    pub kind: ConfigErrorKind,

    /// The id of the contract the error is in, if any.
    pub contract: Option<String>,

    /// Where the error is in the file, when it is known.
    pub location: Option<Location>,
}

/// Where an error is in a configuration file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    /// The 1-based line of the start of the offending key or value.
    pub line: u32,

    /// The 1-based column of the start of the offending key or value, in
    /// characters.
    pub column: u32,

    /// The byte range of the offending key or value.
    pub range: ByteRange,
}

impl Location {
    /// The location of a byte range of `contents`.
    pub(crate) fn new(contents: &str, span: &Range<usize>) -> Self {
        let before = &contents[..contents.floor_char_boundary(span.start)];
        let line_start = before.rfind('\n').map_or(0, |index| index + 1);
        Self {
            line: before.matches('\n').count() as u32 + 1,
            column: before[line_start..].chars().count() as u32 + 1,
            range: ByteRange::new(span.start as u32, span.end as u32),
        }
    }
}

/// What is wrong in a configuration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigErrorKind {
    /// The file is not valid TOML.
    Syntax {
        /// The message of the TOML parser.
        message: String,
    },

    /// A key that zenith doesn't know.
    UnknownKey {
        /// The key as written.
        key: String,

        /// The keys allowed in its table.
        expected: Vec<String>,

        /// The allowed key closest to it, if any is close.
        suggestion: Option<String>,
    },

    /// A required key is missing.
    MissingKey {
        /// The missing key.
        key: String,
    },

    /// A value that is not one of the allowed ones, such as an unknown
    /// `severity`.
    UnknownValue {
        /// The value as written.
        value: String,

        /// The allowed values.
        expected: Vec<String>,

        /// The allowed value closest to it, if any is close.
        suggestion: Option<String>,
    },

    /// A contract `type` that zenith doesn't know.
    UnknownContractType {
        /// The type as written.
        name: String,

        /// The contract types.
        expected: Vec<String>,

        /// The contract type closest to it, if any is close.
        suggestion: Option<String>,
    },

    /// A value of the wrong type, or out of range, as reported by the TOML
    /// deserialiser.
    InvalidValue {
        /// What is wrong, e.g. "invalid type: integer `1`, expected a
        /// string".
        message: String,
    },

    /// A value that must be a table, such as a contract, is not one.
    NotATable {
        /// The dotted key of the value, e.g. `contracts.web-layers`.
        key: String,
    },

    /// A contract `type` is not a string.
    ContractTypeNotAString,

    /// A contract id with characters other than ASCII letters, digits, `-`
    /// and `_`.
    InvalidContractId {
        /// The id as written.
        id: String,
    },

    /// An invalid selector.
    InvalidSelector {
        /// The selector as written.
        selector: String,

        /// What is wrong with it.
        error: SelectorError,
    },

    /// An invalid glob or name pattern outside a selector, e.g. in `[files]
    /// exclude`.
    InvalidPattern {
        /// The pattern as written.
        pattern: String,

        /// What is wrong with it.
        error: GlobError,
    },

    /// An invalid `ignore_imports` entry.
    InvalidIgnoreImport {
        /// The entry as written.
        entry: String,

        /// What is wrong with it.
        error: IgnoreImportError,
    },

    /// An invalid layer of a `layers` contract.
    InvalidLayer {
        /// The layer as written.
        layer: String,

        /// What is wrong with it.
        error: LayerError,
    },

    /// A `layers` contract has no layers.
    NoLayers,

    /// A `layers` contract is `exhaustive` without `containers`, which are
    /// what it would be exhaustive about.
    ExhaustiveWithoutContainers,
}

impl fmt::Display for ConfigErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { message } => write!(f, "invalid TOML: {message}"),
            Self::UnknownKey { key, expected, suggestion } => {
                write!(f, "unknown key `{key}`")?;
                write_expected(f, expected, suggestion.as_deref())
            }
            Self::MissingKey { key } => write!(f, "missing key `{key}`"),
            Self::UnknownValue { value, expected, suggestion } => {
                write!(f, "unknown value `{value}`")?;
                write_expected(f, expected, suggestion.as_deref())
            }
            Self::UnknownContractType { name, expected, suggestion } => {
                write!(f, "unknown contract type `{name}`")?;
                write_expected(f, expected, suggestion.as_deref())
            }
            Self::InvalidValue { message } => f.write_str(message),
            Self::NotATable { key } => write!(f, "`{key}` must be a table"),
            Self::ContractTypeNotAString => f.write_str("`type` must be a string"),
            Self::InvalidContractId { id } => write!(
                f,
                "invalid contract id `{id}`: ids may only contain ASCII letters, digits, `-` and `_`"
            ),
            Self::InvalidSelector { selector, error } => {
                write!(f, "invalid selector `{selector}`: {error}")
            }
            Self::InvalidPattern { pattern, error } => {
                write!(f, "invalid pattern `{pattern}`: {error}")
            }
            Self::InvalidIgnoreImport { entry, error } => {
                write!(f, "invalid `ignore_imports` entry `{entry}`: {error}")
            }
            Self::InvalidLayer { layer, error } => write!(f, "invalid layer `{layer}`: {error}"),
            Self::NoLayers => f.write_str("`layers` must list at least one layer"),
            Self::ExhaustiveWithoutContainers => f.write_str("`exhaustive` requires `containers`"),
        }
    }
}

/// Write a "did you mean" hint, or else the allowed names.
fn write_expected(
    f: &mut fmt::Formatter<'_>,
    expected: &[String],
    suggestion: Option<&str>,
) -> fmt::Result {
    if let Some(suggestion) = suggestion {
        return write!(f, "; did you mean `{suggestion}`?");
    }
    match expected {
        [] => Ok(()),
        [only] => write!(f, ", expected `{only}`"),
        [first, second] => write!(f, ", expected `{first}` or `{second}`"),
        names => {
            let names: Vec<_> = names.iter().map(|name| format!("`{name}`")).collect();
            write!(f, ", expected one of {}", names.join(", "))
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(contract) = &self.contract {
            write!(f, "in contract `{contract}`: ")?;
        }
        write!(f, "{}", self.kind)
    }
}

impl std::error::Error for ConfigError {}

/// Why a configuration file could not be loaded.
#[derive(Debug)]
pub enum LoadError {
    /// The file could not be read.
    Read {
        /// The configuration file.
        path: PathBuf,

        /// Why it could not be read.
        error: io::Error,
    },

    /// The file is invalid.
    Invalid {
        /// The configuration file.
        path: PathBuf,

        /// The contents of the file, for showing the errors in context.
        contents: String,

        /// What is wrong, in the order of the file. There is at least one
        /// error.
        errors: Vec<ConfigError>,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, error } => write!(f, "could not read `{}`: {error}", path.display()),
            Self::Invalid { path, errors, .. } => {
                for (index, error) in errors.iter().enumerate() {
                    if index > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{}", path.display())?;
                    if let Some(location) = &error.location {
                        write!(f, ":{}:{}", location.line, location.column)?;
                    }
                    write!(f, ": {error}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { error, .. } => Some(error),
            Self::Invalid { .. } => None,
        }
    }
}

/// The candidate closest to `unknown`, if any is close enough to be what
/// was meant. Ties go to the earliest candidate.
pub(crate) fn suggest<'a>(
    unknown: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    // The same measure and threshold as clap's "did you mean".
    let mut best: Option<(f64, &str)> = None;
    for candidate in candidates {
        let confidence = strsim::jaro(unknown, candidate);
        if confidence > 0.7 && best.is_none_or(|(highest, _)| confidence > highest) {
            best = Some((confidence, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// Turn an error of the TOML deserialiser into a [`ConfigErrorKind`]. serde
/// reports unknown keys and values, and missing keys, with messages of a
/// fixed form, such as "unknown field `fromm`, expected one of `from`,
/// `to`", which are recognised here to suggest a fix.
pub(crate) fn classify(message: &str) -> ConfigErrorKind {
    if let Some((key, expected)) = quoted_and_expected(message, "unknown field `") {
        let suggestion = suggest(&key, expected.iter().map(String::as_str)).map(String::from);
        return ConfigErrorKind::UnknownKey { key, expected, suggestion };
    }
    if let Some((value, expected)) = quoted_and_expected(message, "unknown variant `") {
        let suggestion = suggest(&value, expected.iter().map(String::as_str)).map(String::from);
        return ConfigErrorKind::UnknownValue { value, expected, suggestion };
    }
    if let Some((key, _)) = quoted_and_expected(message, "missing field `") {
        return ConfigErrorKind::MissingKey { key };
    }
    ConfigErrorKind::InvalidValue { message: tidy_invalid_type(message) }
}

/// Split `<prefix><name>`, expected `a`, `b`` into the name and the allowed
/// names.
fn quoted_and_expected(message: &str, prefix: &str) -> Option<(String, Vec<String>)> {
    let rest = message.strip_prefix(prefix)?;
    // A quoted key may contain a backquote, so the name ends where serde's
    // list of the allowed names starts, or at the end of the message.
    let (name, expected) = match rest.split_once("`, expected ") {
        Some((name, expected)) => (name, expected),
        None => match rest.split_once("`, there are no ") {
            Some((name, _)) => (name, ""),
            None => (rest.strip_suffix('`')?, ""),
        },
    };
    let expected = expected.split('`').skip(1).step_by(2).map(String::from).collect();
    Some((name.to_string(), expected))
}

/// Use TOML's names in serde's "invalid type" messages: arrays and tables,
/// rather than sequences and maps.
fn tidy_invalid_type(message: &str) -> String {
    let Some(rest) = message.strip_prefix("invalid type: ") else {
        return message.to_string();
    };
    // The expectation comes last, and never contains user input.
    let Some((unexpected, expected)) = rest.rsplit_once(", expected ") else {
        return message.to_string();
    };
    let unexpected = match unexpected {
        "sequence" => "array",
        "map" => "table",
        other => other,
    };
    let expected = match expected {
        "a sequence" => "an array",
        "a map" => "a table",
        other => other,
    };
    format!("invalid type: {unexpected}, expected {expected}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_close_names() {
        let keys = ["from", "to", "allow_indirect_imports", "name", "severity"];
        assert_eq!(suggest("fromm", keys), Some("from"));
        assert_eq!(suggest("sevrity", keys), Some("severity"));
        assert_eq!(suggest("allow_indirect", keys), Some("allow_indirect_imports"));
        assert_eq!(suggest("modules", keys), None);
        assert_eq!(suggest("warning", ["error", "warn"]), Some("warn"));
        assert_eq!(suggest("x", ["error", "warn"]), None);
        assert_eq!(suggest("x", []), None);
    }

    #[test]
    fn suggests_the_earliest_of_equally_close_names() {
        assert_eq!(suggest("ab", ["abc", "abd"]), Some("abc"));
        assert_eq!(suggest("ab", ["abd", "abc"]), Some("abd"));
    }

    #[test]
    fn classifies_serde_messages() {
        assert_eq!(
            classify("unknown field `exclud`, expected one of `include`, `exclude`"),
            ConfigErrorKind::UnknownKey {
                key: "exclud".into(),
                expected: vec!["include".into(), "exclude".into()],
                suggestion: Some("exclude".into()),
            }
        );
        assert_eq!(
            classify("unknown field `zzz`, expected `path`"),
            ConfigErrorKind::UnknownKey {
                key: "zzz".into(),
                expected: vec!["path".into()],
                suggestion: None,
            }
        );
        assert_eq!(
            classify("unknown field `a`b`, expected `path`"),
            ConfigErrorKind::UnknownKey {
                key: "a`b".into(),
                expected: vec!["path".into()],
                suggestion: None,
            }
        );
        assert_eq!(
            classify("unknown field `a`, there are no fields"),
            ConfigErrorKind::UnknownKey { key: "a".into(), expected: Vec::new(), suggestion: None }
        );
    }

    #[test]
    fn names_arrays_and_tables_as_toml_does() {
        let tidy = |message| match classify(message) {
            ConfigErrorKind::InvalidValue { message } => message,
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(
            tidy("invalid type: map, expected a boolean"),
            "invalid type: table, expected a boolean"
        );
        // A string that happens to say "map" is left alone.
        assert_eq!(
            tidy("invalid type: string \"map\", expected a boolean"),
            "invalid type: string \"map\", expected a boolean"
        );
    }

    #[test]
    fn shows_suggestions_or_else_the_allowed_names() {
        let unknown = |expected: &[&str], suggestion: Option<&str>| {
            ConfigErrorKind::UnknownKey {
                key: "x".into(),
                expected: expected.iter().map(|name| name.to_string()).collect(),
                suggestion: suggestion.map(String::from),
            }
            .to_string()
        };
        assert_eq!(unknown(&["a", "b"], Some("a")), "unknown key `x`; did you mean `a`?");
        assert_eq!(unknown(&[], None), "unknown key `x`");
        assert_eq!(unknown(&["a"], None), "unknown key `x`, expected `a`");
        assert_eq!(unknown(&["a", "b"], None), "unknown key `x`, expected `a` or `b`");
        assert_eq!(
            unknown(&["a", "b", "c"], None),
            "unknown key `x`, expected one of `a`, `b`, `c`"
        );
    }

    #[test]
    fn locates_byte_ranges() {
        let contents = "[files]\nexclude = [\"é\", 1]\n";
        let location = Location::new(contents, &(25..26));
        assert_eq!((location.line, location.column), (2, 17));
        assert_eq!(
            Location::new(contents, &(0..7)),
            Location { line: 1, column: 1, range: ByteRange::new(0, 7) }
        );
        // The end of the file, where a missing `]` would be.
        let end = Location::new(contents, &(contents.len()..contents.len()));
        assert_eq!((end.line, end.column), (3, 1));
    }
}
