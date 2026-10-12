//! The `zenith-ignore` comments that had no effect.

use rustc_hash::FxHashSet;
use zenith_config::Config;
use zenith_diagnostics::{Report, Source, SourceMap, Span};
use zenith_parse::Suppression;
use zenith_utils::location::Located;

use crate::{ContractReport, Exemptions};

/// A `zenith-ignore` comment that had no effect, which is a warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnusedSuppression<'a> {
    /// The file it is in, as a POSIX path relative to the root.
    pub file: &'a str,

    /// The comment.
    pub suppression: &'a Located<Suppression>,

    /// Why it had no effect.
    pub reason: UnusedReason<'a>,
}

/// Why a `zenith-ignore` comment had no effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnusedReason<'a> {
    /// It names no contract.
    NoContract,

    /// It names contracts that the configuration doesn't have, in the order
    /// it names them.
    UnknownContracts(Vec<&'a str>),

    /// None of the checked contracts that it names counts an import statement
    /// that starts on its target line.
    NoImport,
}

impl<'a> UnusedSuppression<'a> {
    /// The suppressions among those that the project `exempts` which had no
    /// effect on the contracts of `reports`, in the order of their files, then
    /// in source order.
    ///
    /// Every contract of the `config` is known, so a suppression that only
    /// names contracts that weren't checked is not unused.
    pub fn find(
        config: &Config,
        exempts: &Exemptions<'a>,
        reports: &[ContractReport],
    ) -> Vec<Self> {
        let used: FxHashSet<_> =
            reports.iter().flat_map(|report| report.used_suppressions.iter().copied()).collect();
        let checked = |id: &str| reports.iter().any(|report| report.config.id == id);

        let mut unused = Vec::new();
        for (index, file) in exempts.suppressions.iter().enumerate() {
            for (position, suppression) in file.suppressions.iter().enumerate() {
                let contracts = &suppression.value.contracts;
                let unknown: Vec<_> = contracts
                    .iter()
                    .filter(|id| config.contract(id).is_none())
                    .map(String::as_str)
                    .collect();
                let reason = if contracts.is_empty() {
                    UnusedReason::NoContract
                } else if !unknown.is_empty() {
                    UnusedReason::UnknownContracts(unknown)
                } else if !used.contains(&(index, position))
                    && contracts.iter().any(|id| checked(id))
                {
                    UnusedReason::NoImport
                } else {
                    continue;
                };
                unused.push(Self { file: &file.file, suppression, reason });
            }
        }
        unused
    }
}

impl UnusedSuppression<'_> {
    /// The report of the suppression: a warning, pointing at the comment in
    /// `sources` if its file can be read.
    pub fn report(&self, sources: &mut SourceMap) -> Report {
        let source = sources.file(self.file);
        let mut report = Report::warning(self.message(source.map(|id| sources.get(id))));
        if let Some(source) = source {
            report.add_span(Span::new(source, self.suppression.range));
        }
        report
    }

    /// What is wrong with the suppression, naming its target line if its
    /// file's `source` can be read.
    fn message(&self, source: Option<&Source>) -> String {
        match &self.reason {
            UnusedReason::NoContract => "unused suppression: it names no contract".to_string(),
            UnusedReason::UnknownContracts(ids) => {
                let ids: Vec<_> = ids.iter().map(|id| format!("`{id}`")).collect();
                match ids.as_slice() {
                    [id] => format!("unused suppression: there is no contract {id}"),
                    ids => {
                        format!("unused suppression: there are no contracts {}", ids.join(", "))
                    }
                }
            }
            UnusedReason::NoImport => match source {
                Some(source) => {
                    let line = source.line(self.suppression.value.target.start());
                    format!(
                        "unused suppression: no import on line {line} for the contracts it names"
                    )
                }
                None => "unused suppression: no import on its line for the contracts it names"
                    .to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::{Project, values};

    #[test]
    fn suppressions_that_name_nothing_checked_on_their_line_are_unused() {
        let toml = "[contracts.f]
type = \"forbidden\"
from = [\"a\"]
to = [\"b\"]

[contracts.g]
type = \"forbidden\"
from = [\"b\"]
to = [\"a\"]
";
        let project =
            Project::new(&values(&[("a/x.ts", "b/y.ts"), ("a/x.ts", "c/z.ts")]), &[], toml)
                .suppress("a/x.ts", 1, "f")
                // Used by `f`, so `g` finding nothing on the line doesn't matter.
                .suppress("a/x.ts", 1, "f g")
                .suppress("a/x.ts", 2, "")
                .suppress("a/x.ts", 2, "f nope nada")
                // On a line without imports.
                .suppress("a/x.ts", 3, "f");
        let reasons: Vec<_> = project
            .unused_suppressions(&["f", "g"])
            .iter()
            .map(|unused| {
                format!("{}: {:?}", unused.suppression.value.contracts.join(" "), unused.reason)
            })
            .collect();
        assert_eq!(
            reasons,
            [": NoContract", "f nope nada: UnknownContracts([\"nope\", \"nada\"])", "f: NoImport"]
        );
    }

    #[test]
    fn suppressions_of_contracts_that_were_not_checked_are_not_unused() {
        let toml = "[contracts.f]\ntype = \"forbidden\"\nfrom = [\"a\"]\nto = [\"b\"]\n\n[contracts.g]\ntype = \"forbidden\"\nfrom = [\"b\"]\nto = [\"a\"]\n";
        let project =
            Project::new(&values(&[("a/x.ts", "b/y.ts")]), &[], toml).suppress("a/x.ts", 5, "g");
        assert!(project.unused_suppressions(&["f"]).is_empty());
        assert_eq!(project.unused_suppressions(&["f", "g"]).len(), 1);
    }
}
