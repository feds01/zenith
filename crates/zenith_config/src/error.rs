//! Errors in configuration files, with the byte range of the offending key or
//! value, so that they can be shown in context.

use std::{fmt, io, path::PathBuf};

use zenith_diagnostics::{Report, Reporter, SourceId, SourceMap, Span};
use zenith_utils::{glob::GlobError, location::ByteRange};

/// An error in a configuration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    /// What is wrong.
    pub kind: ConfigErrorKind,

    /// The id of the contract the error is in, if any.
    pub contract: Option<String>,

    /// The byte range of the offending key or value, when it is known.
    pub range: Option<ByteRange>,
}

impl ConfigError {
    /// The report of the error, which points into `source`, the
    /// configuration file.
    pub fn report(&self, source: SourceId) -> Report {
        let mut report = Report::error(self.kind.title());
        if let Some(range) = self.range {
            report.add_span(Span::new(source, range));
        }
        if let Some(contract) = &self.contract {
            report.add_note(format!("in contract `{contract}`"));
        }
        if let Some(help) = self.kind.help() {
            report.add_help(help);
        }
        report
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

    /// A contract has `boundaries` and `allow_indirect_imports`, with which
    /// only direct imports count, and they pass through nothing.
    BoundariesWithoutChains,

    /// An `encapsulation` contract sets `reexports = "transparent"`, although
    /// importing through entry points, which often only re-export, is what it
    /// allows.
    TransparentEncapsulation,
}

impl ConfigErrorKind {
    /// What is wrong, without the names that an unknown name could have
    /// been.
    fn title(&self) -> String {
        match self {
            Self::Syntax { message } => format!("invalid TOML: {message}"),
            Self::UnknownKey { key, .. } => format!("unknown key `{key}`"),
            Self::MissingKey { key } => format!("missing key `{key}`"),
            Self::UnknownValue { value, .. } => format!("unknown value `{value}`"),
            Self::UnknownContractType { name, .. } => format!("unknown contract type `{name}`"),
            Self::InvalidValue { message } => message.clone(),
            Self::NotATable { key } => format!("`{key}` must be a table"),
            Self::ContractTypeNotAString => "`type` must be a string".to_string(),
            Self::InvalidContractId { id } => format!(
                "invalid contract id `{id}`: ids may only contain ASCII letters, digits, `-` and `_`"
            ),
            Self::InvalidSelector { selector, error } => {
                format!("invalid selector `{selector}`: {error}")
            }
            Self::InvalidPattern { pattern, error } => {
                format!("invalid pattern `{pattern}`: {error}")
            }
            Self::InvalidIgnoreImport { entry, error } => {
                format!("invalid `ignore_imports` entry `{entry}`: {error}")
            }
            Self::InvalidLayer { layer, error } => format!("invalid layer `{layer}`: {error}"),
            Self::NoLayers => "`layers` must list at least one layer".to_string(),
            Self::ExhaustiveWithoutContainers => "`exhaustive` requires `containers`".to_string(),
            Self::BoundariesWithoutChains => {
                "`boundaries` has no effect with `allow_indirect_imports = true`, since direct \
                 imports pass through nothing"
                    .to_string()
            }
            Self::TransparentEncapsulation => {
                "`encapsulation` contracts can't see through re-exports, since importing through an \
                 entry point, which often only re-exports, is what they allow"
                    .to_string()
            }
        }
    }

    /// The names that an unknown name could have been, and the closest of
    /// them, if any is close.
    fn alternatives(&self) -> Option<(&[String], Option<&str>)> {
        match self {
            Self::UnknownKey { expected, suggestion, .. }
            | Self::UnknownValue { expected, suggestion, .. }
            | Self::UnknownContractType { expected, suggestion, .. } => {
                Some((expected, suggestion.as_deref()))
            }
            _ => None,
        }
    }

    /// How to fix an unknown name: the closest allowed name, or else the
    /// allowed names.
    fn help(&self) -> Option<String> {
        match self.alternatives()? {
            (_, Some(suggestion)) => Some(format!("did you mean `{suggestion}`?")),
            (expected, None) => expected_names(expected).map(|names| format!("expected {names}")),
        }
    }
}

/// The allowed names, as `` `a` ``, `` `a` or `b` `` or
/// `` one of `a`, `b`, `c` ``, or `None` if there are none.
fn expected_names(names: &[String]) -> Option<String> {
    match names {
        [] => None,
        [only] => Some(format!("`{only}`")),
        [first, second] => Some(format!("`{first}` or `{second}`")),
        names => {
            let names: Vec<_> = names.iter().map(|name| format!("`{name}`")).collect();
            Some(format!("one of {}", names.join(", ")))
        }
    }
}

/// On one line: the title, then a "did you mean" hint or the allowed names.
impl fmt::Display for ConfigErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.title())?;
        match self.alternatives() {
            Some((_, Some(suggestion))) => write!(f, "; did you mean `{suggestion}`?"),
            Some((expected, None)) => match expected_names(expected) {
                Some(names) => write!(f, ", expected {names}"),
                None => Ok(()),
            },
            None => Ok(()),
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

/// An invalid file is shown as the reports of its errors, without colour.
impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, error } => write!(f, "could not read `{}`: {error}", path.display()),
            Self::Invalid { path, contents, errors } => {
                let mut sources = SourceMap::new();
                let source = sources.add(path.display().to_string(), contents.as_str());
                let reports: Vec<_> = errors.iter().map(|error| error.report(source)).collect();
                write!(f, "{}", Reporter::new(&sources, &reports))
            }
        }
    }
}

/// The message of a [`LoadError::Read`] already ends with its cause, so the
/// error has no `source`: reporters that show the chain of causes would show
/// it twice.
impl std::error::Error for LoadError {}

/// Why a selector is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorError {
    /// The glob or name pattern is invalid.
    Pattern(GlobError),

    /// A path goes up with `..`, out of the project root.
    ParentDir,

    /// A `directive` is set on a selector that matches no files.
    DirectiveWithoutFiles,
}

impl fmt::Display for SelectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pattern(error) => write!(f, "{error}"),
            Self::ParentDir => {
                f.write_str("`..` is not allowed; paths are relative to the project root")
            }
            Self::DirectiveWithoutFiles => f.write_str(
                "`directive` only applies to path and `pkg:` selectors, which match files",
            ),
        }
    }
}

impl std::error::Error for SelectorError {}

impl From<GlobError> for SelectorError {
    fn from(error: GlobError) -> Self {
        Self::Pattern(error)
    }
}

/// Why an `ignore_imports` entry is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IgnoreImportError {
    /// The entry doesn't have exactly one `->`.
    Arrow,

    /// One of the two selectors is invalid.
    Selector {
        /// The selector as written.
        selector: String,

        /// What is wrong with it.
        error: SelectorError,
    },
}

impl fmt::Display for IgnoreImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Arrow => f.write_str("expected `<importer> -> <imported>`"),
            Self::Selector { selector, error } => {
                write!(f, "invalid selector `{selector}`: {error}")
            }
        }
    }
}

impl std::error::Error for IgnoreImportError {}

/// Why a layer is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerError {
    /// The layer separates its members with both `|` and `:`.
    MixedSeparators,

    /// A member is empty, as in `"a | "`.
    EmptyMember,

    /// A member is not a valid selector.
    Selector {
        /// The member as written.
        member: String,

        /// What is wrong with it.
        error: SelectorError,
    },

    /// The contract has `containers`, but a member is not a path, which
    /// would be relative to each container.
    NotRelative {
        /// The member as written.
        member: String,
    },
}

impl fmt::Display for LayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MixedSeparators => f.write_str("a layer cannot mix `|` and `:` separators"),
            Self::EmptyMember => f.write_str("a member is empty"),
            Self::Selector { member, error } => write!(f, "invalid selector `{member}`: {error}"),
            Self::NotRelative { member } => write!(
                f,
                "`{member}` is not a path; with `containers`, layers are paths relative to each container"
            ),
        }
    }
}

impl std::error::Error for LayerError {}

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
    fn reports_show_the_contract_and_the_fix() {
        let mut sources = SourceMap::new();
        let source = sources.add("zenith.toml", "[contracts.c]\nfromm = []\n");
        let error = ConfigError {
            kind: classify("unknown field `fromm`, expected one of `from`, `to`"),
            contract: Some("c".into()),
            range: Some(ByteRange::new(14, 19)),
        };
        assert_eq!(error.to_string(), "in contract `c`: unknown key `fromm`; did you mean `from`?");

        let expected = [
            "error: unknown key `fromm`",
            " --> zenith.toml:2:1",
            "  |",
            "2 | fromm = []",
            "  | ^^^^^",
            "  |",
            "  = note: in contract `c`",
            "  = help: did you mean `from`?",
        ];
        let reports = [error.report(source)];
        assert_eq!(Reporter::new(&sources, &reports).to_string(), expected.join("\n"));
    }

    #[test]
    fn reports_the_allowed_names_when_none_is_close() {
        let error = ConfigError {
            kind: classify("unknown variant `fatal`, expected `error` or `warn`"),
            contract: None,
            range: None,
        };
        let mut sources = SourceMap::new();
        let report = error.report(sources.add("zenith.toml", ""));
        let expected =
            ["error: unknown value `fatal`", "  |", "  = help: expected `error` or `warn`"];
        assert_eq!(Reporter::new(&sources, &[report]).to_string(), expected.join("\n"));
    }
}
