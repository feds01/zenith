//! The summary of a check: a line for every contract, and the totals.

use std::time::Duration;

use zenith_diagnostics::ReportKind;
use zenith_utils::{
    highlight::{Colour, Modifier, highlight},
    printing::{plural, thousands},
};

/// Whether a contract is kept or broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing breaks it.
    Kept,

    /// Findings break it.
    Broken {
        /// How many findings break it.
        findings: usize,

        /// Whether they are errors, or only warnings.
        kind: ReportKind,
    },
}

/// The line of the summary about one contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryEntry {
    /// The id of the contract.
    pub id: String,

    /// Its type, e.g. `forbidden`.
    pub kind: &'static str,

    /// Whether it is kept.
    pub outcome: Outcome,

    /// More about it, shown after its outcome, e.g. `3 ignored`.
    pub details: Vec<String>,

    /// How long checking it took.
    pub duration: Duration,
}

/// The summary of a check, shown after its reports.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    /// A line for every contract, in the order of the configuration.
    pub entries: Vec<SummaryEntry>,

    /// How many files were checked.
    pub files: usize,

    /// How many imports were checked.
    pub imports: usize,

    /// How long each step took, by name, or `None` to leave timings out,
    /// those of the entries too.
    pub timings: Option<Vec<(&'static str, Duration)>>,
}

impl Summary {
    /// Whether the check fails: a contract is broken by errors.
    pub fn failed(&self) -> bool {
        self.broken(ReportKind::Error) > 0
    }

    /// A line for every entry, with its ids and types aligned.
    pub(crate) fn entry_lines(&self) -> Vec<String> {
        let width = |of: fn(&SummaryEntry) -> usize| self.entries.iter().map(of).max();
        let id_width = width(|entry| entry.id.len()).unwrap_or(0);
        let kind_width = width(|entry| entry.kind.len()).unwrap_or(0);
        self.entries
            .iter()
            .map(|entry| {
                let (mark, status) = match entry.outcome {
                    Outcome::Kept => (highlight(Colour::Green, "✓"), "kept".to_string()),
                    Outcome::Broken { findings, kind: ReportKind::Error } => {
                        let findings = plural(findings, "violation");
                        (highlight(Colour::Red, "✗"), format!("broken ({findings})"))
                    }
                    Outcome::Broken { findings, kind: ReportKind::Warning } => {
                        let findings = plural(findings, "violation");
                        (
                            highlight(Colour::Yellow, "!"),
                            format!("broken ({findings}, warning only)"),
                        )
                    }
                };
                let details = match entry.details.as_slice() {
                    [] => String::new(),
                    details => format!(" · {}", details.join(", ")),
                };
                let timing = match self.timings {
                    Some(_) => highlight(Modifier::Dim, format!(" · {:.1?}", entry.duration)),
                    None => String::new(),
                };
                let (id, kind) = (&entry.id, entry.kind);
                format!("{mark} {id:<id_width$}  {kind:<kind_width$}  {status}{details}{timing}")
            })
            .collect()
    }

    /// `<kept> kept, <broken> broken[, <n> warnings] · <files> files,
    /// <imports> imports`, followed by the timings if they are shown.
    pub(crate) fn totals(&self) -> String {
        let kept = self.entries.iter().filter(|entry| entry.outcome == Outcome::Kept).count();
        let mut line = format!("{kept} kept, {} broken", self.broken(ReportKind::Error));
        let warnings = self.broken(ReportKind::Warning);
        if warnings > 0 {
            line += &format!(", {}", plural(warnings, "warning"));
        }
        let count = |count: usize, noun: &str| match count {
            1 => format!("1 {noun}"),
            count => format!("{} {noun}s", thousands(count)),
        };
        line += &format!(" · {}, {}", count(self.files, "file"), count(self.imports, "import"));
        if let Some(timings) = &self.timings {
            let steps: Vec<_> =
                timings.iter().map(|(name, time)| format!("{name} {time:.1?}")).collect();
            line += &format!(" · {}", steps.join(", "));
        }
        line
    }

    /// How many contracts are broken by findings of `kind`.
    fn broken(&self, kind: ReportKind) -> usize {
        let broken = |entry: &&SummaryEntry| match entry.outcome {
            Outcome::Broken { kind: of, .. } => of == kind,
            Outcome::Kept => false,
        };
        self.entries.iter().filter(broken).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An entry for the contract `id` of type `kind`.
    fn entry(id: &str, kind: &'static str, outcome: Outcome) -> SummaryEntry {
        SummaryEntry {
            id: id.to_string(),
            kind,
            outcome,
            details: Vec::new(),
            duration: Duration::ZERO,
        }
    }

    /// A summary of `entries`, of 1,234 files and 1 import.
    fn summary(entries: Vec<SummaryEntry>) -> Summary {
        Summary { entries, files: 1_234, imports: 1, timings: None }
    }

    #[test]
    fn only_contracts_broken_by_errors_fail_the_check() {
        let kept = entry("kept", "forbidden", Outcome::Kept);
        let warned = entry(
            "warned",
            "forbidden",
            Outcome::Broken { findings: 2, kind: ReportKind::Warning },
        );
        let broken =
            entry("broken", "layers", Outcome::Broken { findings: 1, kind: ReportKind::Error });

        let only_kept = summary(vec![kept.clone()]);
        assert_eq!(only_kept.totals(), "1 kept, 0 broken · 1,234 files, 1 import");
        assert!(!only_kept.failed());
        let warnings = summary(vec![kept.clone(), warned.clone()]);
        assert_eq!(warnings.totals(), "1 kept, 0 broken, 1 warning · 1,234 files, 1 import");
        assert!(!warnings.failed());
        let errors = summary(vec![kept, warned, broken]);
        assert_eq!(errors.totals(), "1 kept, 1 broken, 1 warning · 1,234 files, 1 import");
        assert!(errors.failed());
    }

    #[test]
    fn entries_are_aligned_by_their_widest_id_and_type() {
        let entries = vec![
            entry("a", "forbidden", Outcome::Kept),
            entry("bcd", "layers", Outcome::Broken { findings: 2, kind: ReportKind::Warning }),
        ];
        assert_eq!(
            summary(entries).entry_lines(),
            ["✓ a    forbidden  kept", "! bcd  layers     broken (2 violations, warning only)",]
        );
    }

    #[test]
    fn details_follow_the_outcome_and_timings_follow_both() {
        let mut detailed = entry("a", "acyclic", Outcome::Kept);
        detailed.details = vec!["3 ignored".to_string(), "1 suppressed".to_string()];
        let mut timed = summary(vec![detailed]);
        timed.timings = Some(vec![("parse", Duration::from_millis(3))]);
        assert_eq!(timed.entry_lines(), ["✓ a  acyclic  kept · 3 ignored, 1 suppressed · 0.0ns"]);
        assert_eq!(timed.totals(), "1 kept, 0 broken · 1,234 files, 1 import · parse 3.0ms");
    }
}
