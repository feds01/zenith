//! The summary of a check: a line for every contract, and the totals.

use std::time::Duration;

use zenith_diagnostics::{ReportKind, message};
use zenith_utils::highlight::{Colour, Modifier, highlight};

/// Whether a contract is kept or broken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing breaks it.
    Kept,

    /// Nothing breaks it, but something about it needs fixing.
    KeptWith {
        /// What to show, e.g. `kept, with a stale baseline`.
        status: String,

        /// Whether it is an error, or only a warning.
        kind: ReportKind,
    },

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

/// A total of a check besides those of the outcomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Count {
    /// The count and what it counts, as the totals line shows it, e.g. `3
    /// stale baseline entries`.
    pub text: String,

    /// How many there are. The totals line leaves out a count of none.
    pub count: usize,
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

    /// Totals besides those of the outcomes, e.g. of stale baseline entries.
    pub counts: Vec<Count>,

    /// How long each step took, by name, or `None` to leave timings out,
    /// those of the entries too.
    pub timings: Option<Vec<(&'static str, Duration)>>,
}

impl Summary {
    /// Whether the check fails: a contract is broken by errors, or kept with
    /// something to fix that is an error.
    pub fn failed(&self) -> bool {
        let flagged = |entry: &SummaryEntry| {
            matches!(entry.outcome, Outcome::KeptWith { kind: ReportKind::Error, .. })
        };
        self.broken(ReportKind::Error) > 0 || self.entries.iter().any(flagged)
    }

    /// A line for every entry, with its ids and types aligned.
    pub(crate) fn entry_lines(&self) -> Vec<String> {
        let width = |of: fn(&SummaryEntry) -> usize| self.entries.iter().map(of).max();
        let id_width = width(|entry| entry.id.len()).unwrap_or(0);
        let kind_width = width(|entry| entry.kind.len()).unwrap_or(0);
        self.entries
            .iter()
            .map(|entry| {
                let (mark, status) = match &entry.outcome {
                    Outcome::Kept => (highlight(Colour::Green, "✓"), message!(reporting_kept)),
                    Outcome::KeptWith { status, kind: ReportKind::Error } => {
                        (highlight(Colour::Red, "✗"), status.clone())
                    }
                    Outcome::KeptWith { status, kind: ReportKind::Warning } => {
                        (highlight(Colour::Yellow, "!"), status.clone())
                    }
                    Outcome::Broken { findings, kind: ReportKind::Error } => (
                        highlight(Colour::Red, "✗"),
                        message!(reporting_broken, findings = *findings),
                    ),
                    Outcome::Broken { findings, kind: ReportKind::Warning } => (
                        highlight(Colour::Yellow, "!"),
                        message!(reporting_broken_warning, findings = *findings),
                    ),
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

    /// `<kept> kept, <broken> broken[, <n> warnings][ · <count>]… · <files>
    /// files, <imports> imports`, followed by the timings if they are shown.
    pub(crate) fn totals(&self) -> String {
        let (kept, broken) = (self.kept(), self.broken(ReportKind::Error));
        let mut line = match self.broken(ReportKind::Warning) {
            0 => message!(reporting_outcomes, kept, broken),
            warnings => message!(reporting_outcomes_with_warnings, kept, broken, warnings),
        };
        for Count { text, count } in &self.counts {
            if *count > 0 {
                line += &format!(" · {text}");
            }
        }
        let checked = message!(reporting_checked, files = self.files, imports = self.imports);
        line += &format!(" · {checked}");
        if let Some(timings) = &self.timings {
            let steps: Vec<_> =
                timings.iter().map(|(name, time)| format!("{name} {time:.1?}")).collect();
            line += &format!(" · {}", steps.join(", "));
        }
        line
    }

    /// How many contracts are kept, with something to fix or not.
    pub fn kept(&self) -> usize {
        let kept = |entry: &&SummaryEntry| {
            matches!(entry.outcome, Outcome::Kept | Outcome::KeptWith { .. })
        };
        self.entries.iter().filter(kept).count()
    }

    /// How many contracts are broken by findings of `kind`.
    pub fn broken(&self, kind: ReportKind) -> usize {
        let broken = |entry: &&SummaryEntry| match entry.outcome {
            Outcome::Broken { kind: of, .. } => of == kind,
            Outcome::Kept | Outcome::KeptWith { .. } => false,
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
        Summary { entries, files: 1_234, imports: 1, counts: Vec::new(), timings: None }
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
    fn contracts_kept_with_errors_fail_the_check() {
        let flagged = |kind| {
            let status = "kept, with a stale baseline".to_string();
            entry("a", "forbidden", Outcome::KeptWith { status, kind })
        };
        let mut warned = summary(vec![flagged(ReportKind::Warning)]);
        let count = |count, text: &str| Count { text: text.to_string(), count };
        warned.counts = vec![count(1, "1 stale baseline entry")];
        assert_eq!(
            warned.totals(),
            "1 kept, 0 broken · 1 stale baseline entry · 1,234 files, 1 import"
        );
        assert_eq!(warned.entry_lines(), ["! a  forbidden  kept, with a stale baseline"]);
        assert!(!warned.failed());
        assert!(summary(vec![flagged(ReportKind::Error)]).failed());
        warned.counts = vec![count(0, "0 stale baseline entries")];
        assert_eq!(warned.totals(), "1 kept, 0 broken · 1,234 files, 1 import");
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
