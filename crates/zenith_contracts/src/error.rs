//! Errors in the contracts of a configuration that only show once its
//! selectors are resolved against the project, such as a selector that
//! matches nothing.

use std::fmt;

use zenith_diagnostics::{Report, SourceId, Span};
use zenith_utils::location::ByteRange;

/// An error in a contract of the configuration file, found when compiling it
/// against the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractError {
    /// The id of the contract.
    pub contract: String,

    /// What is wrong.
    pub kind: ContractErrorKind,

    /// The byte range in the configuration file of the selector that is
    /// wrong, when it was read from a file.
    pub range: Option<ByteRange>,
}

/// What is wrong in a contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractErrorKind {
    /// A path or `pkg:` selector matches nothing, which is likely a typo or a
    /// rename.
    UnmatchedSelector {
        /// The selector, as written.
        selector: String,
    },

    /// Two modules overlap where the contract needs them apart: the modules
    /// of an `independence` contract, or the layers of a `layers` contract.
    OverlappingModules {
        /// The label of the first module.
        first: String,

        /// The label of the second module, which shares members with the
        /// first.
        second: String,
    },

    /// A container of a `layers` contract lacks a layer that isn't optional.
    MissingLayer {
        /// The layer's member, as written.
        member: String,

        /// The label of the container.
        container: String,
    },

    /// A module that must be a directory is not one: a container of a
    /// `layers` contract, or a module of an `encapsulation` contract.
    NotADirectory {
        /// The label of the module.
        module: String,
    },
}

impl ContractError {
    /// The report of the error, which points into `source`, the
    /// configuration file.
    pub fn report(&self, source: SourceId) -> Report {
        let mut report = Report::error(self.kind.to_string());
        if let Some(range) = self.range {
            report.add_span(Span::new(source, range));
        }
        report.add_note(format!("in contract `{}`", self.contract));
        if let Some(help) = self.kind.help() {
            report.add_help(help);
        }
        report
    }
}

impl ContractErrorKind {
    /// How to fix the error, if there is more to say than what is wrong.
    fn help(&self) -> Option<&'static str> {
        match self {
            Self::UnmatchedSelector { .. } => {
                Some("check it for a typo, or for a module that was renamed or removed")
            }
            Self::OverlappingModules { .. } => {
                Some("a module can only be in one of them; narrow one with `except`")
            }
            Self::MissingLayer { .. } => Some("wrap the layer in parentheses to make it optional"),
            Self::NotADirectory { .. } => None,
        }
    }
}

impl fmt::Display for ContractErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnmatchedSelector { selector } => {
                write!(f, "`{selector}` does not match anything")
            }
            Self::OverlappingModules { first, second } => {
                write!(f, "`{first}` and `{second}` overlap")
            }
            Self::MissingLayer { member, container } => {
                write!(f, "`{container}` has no layer `{member}`")
            }
            Self::NotADirectory { module } => write!(f, "`{module}` is not a directory"),
        }
    }
}

impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "in contract `{}`: {}", self.contract, self.kind)
    }
}

impl std::error::Error for ContractError {}

#[cfg(test)]
mod tests {
    use zenith_diagnostics::{Reporter, SourceMap};

    use super::*;

    #[test]
    fn errors_point_at_their_selector() {
        let contents =
            "[contracts.a]\ntype = \"forbidden\"\nfrom = [\"web/srcs\"]\nto = [\"npm:x\"]\n";
        let mut sources = SourceMap::new();
        let source = sources.add("zenith.toml", contents);
        let start = contents.find("\"web/srcs\"").unwrap() as u32;
        let error = ContractError {
            contract: "a".to_string(),
            kind: ContractErrorKind::UnmatchedSelector { selector: "web/srcs".to_string() },
            range: Some(ByteRange::new(start, start + 10)),
        };

        let expected = [
            "error: `web/srcs` does not match anything",
            " --> zenith.toml:3:9",
            "  |",
            "3 | from = [\"web/srcs\"]",
            "  |         ^^^^^^^^^^",
            "  |",
            "  = note: in contract `a`",
            "  = help: check it for a typo, or for a module that was renamed or removed",
        ];
        assert_eq!(
            Reporter::new(&sources, &[error.report(source)]).to_string(),
            expected.join("\n")
        );
        assert_eq!(error.to_string(), "in contract `a`: `web/srcs` does not match anything");
    }
}
