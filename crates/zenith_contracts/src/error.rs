//! Errors in the contracts of a configuration that only show once its
//! selectors are resolved against the project, such as a selector that
//! matches nothing.

use std::fmt;

use zenith_config::InContract;
use zenith_diagnostics::{Diagnostic, message};
use zenith_utils::location::ByteRange;

/// An error in a contract of the configuration file, found when compiling it
/// against the project. Its report is made in the source of the
/// configuration file, which its range is in: the excerpt, then the
/// contract, then how to fix the error.
#[derive(Debug, Clone, PartialEq, Eq, Diagnostic)]
pub struct ContractError {
    /// The contract the error is in.
    #[subdiagnostic]
    pub contract: InContract,

    /// What is wrong.
    #[diag]
    pub kind: ContractErrorKind,

    /// The byte range in the configuration file of the selector that is
    /// wrong, when it was read from a file.
    #[primary_span]
    pub range: Option<ByteRange>,
}

/// What is wrong in a contract.
#[derive(Debug, Clone, PartialEq, Eq, Diagnostic)]
pub enum ContractErrorKind {
    /// A path or `pkg:` selector matches nothing, which is likely a typo or a
    /// rename.
    #[diag(contracts_unmatched_selector)]
    #[help]
    UnmatchedSelector {
        /// The selector, as written.
        selector: String,
    },

    /// Two modules overlap where the contract needs them apart: the modules
    /// of an `independence` contract, or the layers of a `layers` contract.
    #[diag(contracts_overlapping_modules)]
    #[help]
    OverlappingModules {
        /// The label of the first module.
        first: String,

        /// The label of the second module, which shares members with the
        /// first.
        second: String,
    },

    /// A container of a `layers` contract lacks a layer that isn't optional.
    #[diag(contracts_missing_layer)]
    #[help]
    MissingLayer {
        /// The layer's member, as written.
        member: String,

        /// The label of the container.
        container: String,
    },

    /// A module that must be a directory is not one: a container of a
    /// `layers` contract, or a module of an `encapsulation` contract.
    #[diag(contracts_not_a_directory)]
    NotADirectory {
        /// The label of the module.
        module: String,
    },
}

impl fmt::Display for ContractErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.title())
    }
}

impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let InContract(contract) = &self.contract;
        f.write_str(&message!(config_in_contract_inline, contract, error = self.kind))
    }
}

impl std::error::Error for ContractError {}

#[cfg(test)]
mod tests {
    use zenith_diagnostics::{ReportKind, Reporter, SourceMap};

    use super::*;

    #[test]
    fn errors_point_at_their_selector() {
        let contents =
            "[contracts.a]\ntype = \"forbidden\"\nfrom = [\"web/srcs\"]\nto = [\"npm:x\"]\n";
        let mut sources = SourceMap::new();
        let source = sources.add("zenith.toml", contents);
        let start = contents.find("\"web/srcs\"").unwrap() as u32;
        let error = ContractError {
            contract: InContract("a".to_string()),
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
        let report = error.report_in(ReportKind::Error, Some(source));
        assert_eq!(Reporter::new(&sources, &[report]).to_string(), expected.join("\n"));
        assert_eq!(error.to_string(), "in contract `a`: `web/srcs` does not match anything");
    }
}
