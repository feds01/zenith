//! Errors in configuration files, with the byte range of the offending key or
//! value, so that they can be shown in context.

use std::{fmt, io, path::PathBuf};

use zenith_diagnostics::{
    Diagnostic, Report, Reporter, SourceId, SourceMap, Span, Subdiagnostic, message,
};
use zenith_utils::{glob::GlobError, location::ByteRange};

use crate::language::Languages;

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
            report.add_note(message!(config_in_contract, contract));
        }
        self.kind.add_to(&mut report);
        report
    }
}

/// What is wrong in a configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Diagnostic)]
pub enum ConfigErrorKind {
    /// The file is not valid TOML.
    #[diag(config_syntax)]
    Syntax {
        /// The message of the TOML parser.
        message: String,
    },

    /// A key that zenith doesn't know.
    #[diag(config_unknown_key)]
    UnknownKey {
        /// The key as written.
        key: String,

        /// The keys allowed in its table, and the closest to it.
        #[subdiagnostic]
        alternatives: Alternatives,
    },

    /// A required key is missing.
    #[diag(config_missing_key)]
    MissingKey {
        /// The missing key.
        key: String,
    },

    /// A value that is not one of the allowed ones, such as an unknown
    /// `severity`.
    #[diag(config_unknown_value)]
    UnknownValue {
        /// The value as written.
        value: String,

        /// The allowed values, and the closest to it.
        #[subdiagnostic]
        alternatives: Alternatives,
    },

    /// A contract `type` that zenith doesn't know.
    #[diag(config_unknown_contract_type)]
    UnknownContractType {
        /// The type as written.
        name: String,

        /// The contract types, and the closest to it.
        #[subdiagnostic]
        alternatives: Alternatives,
    },

    /// A language table in `[languages]` that zenith doesn't know.
    #[diag(config_unknown_language)]
    UnknownLanguage {
        /// The name as written.
        name: String,

        /// The names that a language's table goes by, and the closest to it.
        #[subdiagnostic]
        alternatives: Alternatives,
    },

    /// A language's table is given under two of its names, such as `js` and
    /// `typescript`.
    #[diag(config_duplicate_language)]
    #[help]
    DuplicateLanguage {
        /// The name of the second table, as written.
        name: String,

        /// The name of the first, as written.
        first: String,
    },

    /// `[languages]` has no language, so there is nothing to scan.
    #[diag(config_no_languages)]
    NoLanguages {
        /// The name of a language to suggest, if there is one.
        #[help]
        example: Option<String>,
    },

    /// A selector names a scheme of a language that is not enabled.
    #[diag(config_disabled_scheme)]
    #[help]
    DisabledScheme {
        /// The selector as written.
        selector: String,

        /// The scheme it starts with, without the `:`.
        scheme: String,

        /// The name of the language that owns the scheme.
        language: String,
    },

    /// A value of the wrong type, or out of range, as reported by the TOML
    /// deserialiser.
    #[diag(config_invalid_value)]
    InvalidValue {
        /// What is wrong, e.g. "invalid type: integer `1`, expected a
        /// string".
        message: String,
    },

    /// A value that must be a table, such as a contract, is not one.
    #[diag(config_not_a_table)]
    NotATable {
        /// The dotted key of the value, e.g. `contracts.web-layers`.
        key: String,
    },

    /// A contract `type` is not a string.
    #[diag(config_contract_type_not_a_string)]
    ContractTypeNotAString,

    /// A contract id with characters other than ASCII letters, digits, `-`
    /// and `_`.
    #[diag(config_invalid_contract_id)]
    InvalidContractId {
        /// The id as written.
        id: String,
    },

    /// An invalid selector.
    #[diag(config_invalid_selector)]
    InvalidSelector {
        /// The selector as written.
        selector: String,

        /// What is wrong with it.
        error: SelectorError,
    },

    /// An invalid glob or name pattern outside a selector, e.g. in `[files]
    /// exclude`.
    #[diag(config_invalid_pattern)]
    InvalidPattern {
        /// The pattern as written.
        pattern: String,

        /// What is wrong with it.
        error: PatternError,
    },

    /// An invalid `ignore_imports` entry.
    #[diag(config_invalid_ignore_import)]
    InvalidIgnoreImport {
        /// The entry as written.
        entry: String,

        /// What is wrong with it.
        error: IgnoreImportError,
    },

    /// An invalid layer of a `layers` contract.
    #[diag(config_invalid_layer)]
    InvalidLayer {
        /// The layer as written.
        layer: String,

        /// What is wrong with it.
        error: LayerError,
    },

    /// A `layers` contract has no layers.
    #[diag(config_no_layers)]
    NoLayers,

    /// A `layers` contract is `exhaustive` without `containers`, which are
    /// what it would be exhaustive about.
    #[diag(config_exhaustive_without_containers)]
    ExhaustiveWithoutContainers,

    /// A contract has `boundaries` and `allow_indirect_imports`, with which
    /// only direct imports count, and they pass through nothing.
    #[diag(config_boundaries_without_chains)]
    BoundariesWithoutChains,

    /// An `encapsulation` contract sets `reexports = "transparent"`, although
    /// importing through entry points, which often only re-export, is what it
    /// allows.
    #[diag(config_transparent_encapsulation)]
    TransparentEncapsulation,
}

impl ConfigErrorKind {
    /// The names that an unknown name could have been, if it is one.
    fn alternatives(&self) -> Option<&Alternatives> {
        match self {
            Self::UnknownKey { alternatives, .. }
            | Self::UnknownValue { alternatives, .. }
            | Self::UnknownLanguage { alternatives, .. }
            | Self::UnknownContractType { alternatives, .. } => Some(alternatives),
            _ => None,
        }
    }
}

/// The names that an unknown name could have been, and the closest of them,
/// if any is close.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alternatives {
    /// The allowed names.
    pub expected: Vec<String>,

    /// The allowed name closest to the unknown one, if any is close.
    pub suggestion: Option<String>,
}

impl Alternatives {
    /// The names that `unknown` could have been, of the `expected` ones.
    pub(crate) fn new(unknown: &str, expected: Vec<String>) -> Self {
        let suggestion = suggest(unknown, expected.iter().map(String::as_str)).map(String::from);
        Self { expected, suggestion }
    }

    /// The allowed names, quoted, as `` `a` ``, `` `a` or `b` `` or
    /// `` one of `a`, `b`, `c` ``, or `None` if there are none.
    fn expected_names(&self) -> Option<String> {
        let names: Vec<_> = self.expected.iter().map(|name| format!("`{name}`")).collect();
        match names.as_slice() {
            [] => None,
            [only] => Some(only.clone()),
            [first, second] => Some(message!(config_two_names, first, second)),
            names => Some(message!(config_many_names, names = names.join(", "))),
        }
    }

    /// How to fix the unknown name: the closest allowed name, or else the
    /// allowed names, if there are any.
    fn help(&self) -> Option<String> {
        match &self.suggestion {
            Some(suggestion) => Some(message!(config_did_you_mean, suggestion)),
            None => self.expected_names().map(|names| message!(config_expected, names)),
        }
    }
}

impl Alternatives {
    /// `title`, then on the same line how to fix the unknown name, as
    /// [`help`](Self::help) gives it.
    fn inline(&self, title: String) -> String {
        match (&self.suggestion, self.expected_names()) {
            (Some(suggestion), _) => message!(config_did_you_mean_inline, title, suggestion),
            (None, Some(names)) => message!(config_expected_inline, title, names),
            (None, None) => title,
        }
    }

    /// [`inline`](Self::inline) for schemes: each is written with its `:`,
    /// and the allowed ones are always "one of" them.
    fn inline_schemes(&self, title: String) -> String {
        match &self.suggestion {
            Some(suggestion) => {
                let suggestion = format!("{suggestion}:");
                message!(config_did_you_mean_inline, title, suggestion)
            }
            None => {
                let names: Vec<_> = self.expected.iter().map(|name| format!("`{name}:`")).collect();
                let names = message!(config_many_names, names = names.join(", "));
                message!(config_expected_inline, title, names)
            }
        }
    }
}

impl Subdiagnostic for Alternatives {
    fn add_to(&self, report: &mut Report) {
        if let Some(help) = self.help() {
            report.add_help(help);
        }
    }
}

/// On one line: the title, then a "did you mean" hint or the allowed names.
impl fmt::Display for ConfigErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let title = self.title();
        match self.alternatives() {
            Some(alternatives) => f.write_str(&alternatives.inline(title)),
            None => f.write_str(&title),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.contract {
            Some(contract) => {
                f.write_str(&message!(config_in_contract_inline, contract, error = self.kind))
            }
            None => write!(f, "{}", self.kind),
        }
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
            Self::Read { path, error } => {
                f.write_str(&message!(config_read, path = path.display(), error))
            }
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

/// Why a glob or name pattern is invalid, shown with the catalogue's text.
/// The text doesn't repeat the pattern, so callers can name it with its
/// context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatternError(pub GlobError);

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&match self.0 {
            GlobError::Empty => message!(config_glob_empty),
            GlobError::EmptyPath => message!(config_glob_empty_path),
            GlobError::EmptySegment => message!(config_glob_empty_segment),
            GlobError::LeadingSlash => message!(config_glob_leading_slash),
            GlobError::TrailingSlash => message!(config_glob_trailing_slash),
            GlobError::EscapedSlash => message!(config_glob_escaped_slash),
            GlobError::TrailingBackslash => message!(config_glob_trailing_backslash),
            GlobError::DoubleStar => message!(config_glob_double_star),
            GlobError::UnclosedBrace => message!(config_glob_unclosed_brace),
        })
    }
}

impl std::error::Error for PatternError {}

/// Why a selector is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorError {
    /// The glob or name pattern is invalid.
    Pattern(GlobError),

    /// The selector starts with a scheme that zenith doesn't know, as in
    /// `nmp:react`.
    UnknownScheme {
        /// The scheme as written, without the `:`.
        scheme: String,

        /// The schemes that zenith knows, without the `:`, and the closest
        /// to it.
        alternatives: Alternatives,
    },

    /// A path goes up with `..`, out of the project root.
    ParentDir,

    /// A `directive` is set on a selector that matches no files.
    DirectiveWithoutFiles,
}

impl fmt::Display for SelectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pattern(error) => write!(f, "{}", PatternError(*error)),
            Self::UnknownScheme { scheme, alternatives } => {
                f.write_str(&alternatives.inline_schemes(message!(config_unknown_scheme, scheme)))
            }
            Self::ParentDir => f.write_str(&message!(config_parent_dir)),
            Self::DirectiveWithoutFiles => f.write_str(&message!(config_directive_without_files)),
        }
    }
}

impl std::error::Error for SelectorError {}

impl SelectorError {
    /// The error for a selector that starts with `scheme`, which none of the
    /// `languages` knows.
    pub(crate) fn unknown_scheme(scheme: &str, languages: Languages) -> Self {
        let alternatives =
            Alternatives::new(scheme, languages.schemes().map(String::from).collect());
        Self::UnknownScheme { scheme: scheme.to_string(), alternatives }
    }
}

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
            Self::Arrow => f.write_str(&message!(config_ignore_import_arrow)),
            Self::Selector { selector, error } => {
                f.write_str(&message!(config_invalid_selector, selector, error))
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
            Self::MixedSeparators => f.write_str(&message!(config_mixed_separators)),
            Self::EmptyMember => f.write_str(&message!(config_empty_member)),
            Self::Selector { member, error } => {
                f.write_str(&message!(config_invalid_selector, selector = member, error))
            }
            Self::NotRelative { member } => f.write_str(&message!(config_not_relative, member)),
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
        let alternatives = Alternatives::new(&key, expected);
        return ConfigErrorKind::UnknownKey { key, alternatives };
    }
    if let Some((value, expected)) = quoted_and_expected(message, "unknown variant `") {
        let alternatives = Alternatives::new(&value, expected);
        return ConfigErrorKind::UnknownValue { value, alternatives };
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
        "sequence" => message!(config_value_array),
        "map" => message!(config_value_table),
        other => other.to_string(),
    };
    message!(config_invalid_type, unexpected, expected = expectation(expected))
}

/// The catalogue's text for what serde `expected`: one of the expectations
/// that the tables of the schema declare with `expecting`, which are matched
/// here as they are written there, or serde's names for an array or a
/// table. serde's names for other types, such as "a boolean", are its own.
fn expectation(expected: &str) -> String {
    match expected {
        "a sequence" => message!(config_type_array),
        "a map" | "a table" => message!(config_type_table),
        "a contract table" => message!(config_type_contract_table),
        "a selector table" => message!(config_type_selector_table),
        "a selector string or table" => message!(config_type_selector),
        "a layer table" => message!(config_type_layer_table),
        "a layer string or table" => message!(config_type_layer),
        other => other.to_string(),
    }
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
                alternatives: Alternatives {
                    expected: vec!["include".into(), "exclude".into()],
                    suggestion: Some("exclude".into()),
                },
            }
        );
        assert_eq!(
            classify("unknown field `zzz`, expected `path`"),
            ConfigErrorKind::UnknownKey {
                key: "zzz".into(),
                alternatives: Alternatives { expected: vec!["path".into()], suggestion: None },
            }
        );
        assert_eq!(
            classify("unknown field `a`b`, expected `path`"),
            ConfigErrorKind::UnknownKey {
                key: "a`b".into(),
                alternatives: Alternatives { expected: vec!["path".into()], suggestion: None },
            }
        );
        assert_eq!(
            classify("unknown field `a`, there are no fields"),
            ConfigErrorKind::UnknownKey {
                key: "a".into(),
                alternatives: Alternatives { expected: Vec::new(), suggestion: None },
            }
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
    fn pattern_errors_explain_the_fix() {
        assert_eq!(
            PatternError(GlobError::LeadingSlash).to_string(),
            "leading `/`; globs are relative to the project root"
        );
        assert_eq!(
            PatternError(GlobError::DoubleStar).to_string(),
            "`**` must be a whole path segment; use `*` within a segment"
        );
    }

    #[test]
    fn shows_suggestions_or_else_the_allowed_names() {
        let unknown = |expected: &[&str], suggestion: Option<&str>| {
            let expected = expected.iter().map(|name| name.to_string()).collect();
            let alternatives = Alternatives { expected, suggestion: suggestion.map(String::from) };
            ConfigErrorKind::UnknownKey { key: "x".into(), alternatives }.to_string()
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
