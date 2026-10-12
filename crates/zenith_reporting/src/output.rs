//! Showing the reports and the summary of a check in a format.

use std::fmt;

use zenith_diagnostics::{Report, ReportElement, ReportKind, ReportNoteKind, Reporter, SourceMap};
use zenith_utils::highlight::{Modifier, colour_enabled, highlight};

use crate::Summary;

/// How to show the output of a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Every report with excerpts, then a line for every contract, and the
    /// totals.
    Pretty,

    /// A line for every report, `path:line:col: level[contract]: message`,
    /// then the totals.
    Compact,

    /// A GitHub Actions workflow command for every report, which shows it as
    /// an annotation on its line of a pull request, then the totals.
    Github,
}

/// The output of a check, shown in its [`Format`].
pub struct Output<'a> {
    /// The sources that the reports point into.
    pub sources: &'a SourceMap,

    /// The reports of what is wrong, in the order to show them.
    pub reports: &'a [Report],

    /// The summary of every contract.
    pub summary: &'a Summary,

    /// How to show them.
    pub format: Format,
}

impl fmt::Display for Output<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reporter = Reporter::new(self.sources, self.reports).with_colour(colour_enabled());
        match self.format {
            Format::Pretty => {
                if !self.reports.is_empty() {
                    write!(f, "{reporter}\n\n")?;
                }
                writeln!(f, "{}", highlight(Modifier::Bold, "Contracts"))?;
                for line in self.summary.entry_lines() {
                    writeln!(f, "  {line}")?;
                }
                write!(f, "\n{}", self.summary.totals())
            }
            Format::Compact => {
                if !self.reports.is_empty() {
                    writeln!(f, "{}", reporter.short())?;
                }
                write!(f, "{}", self.summary.totals())
            }
            Format::Github => {
                for report in self.reports {
                    writeln!(f, "{}", workflow_command(self.sources, report))?;
                }
                write!(f, "{}", self.summary.totals())
            }
        }
    }
}

/// The workflow command of `report`: `::error` or `::warning`, with where its
/// first excerpt starts, and its title followed by its help, such as the
/// contract's guidance.
fn workflow_command(sources: &SourceMap, report: &Report) -> String {
    let mut properties = Vec::new();
    let span = report.contents.iter().find_map(|element| match element {
        ReportElement::CodeBlock(block) => Some(block.span),
        ReportElement::Note(_) => None,
    });
    if let Some(span) = span {
        let location = sources.location(span);
        properties.push(format!("file={}", escape_property(location.path)));
        properties.push(format!("line={}", location.position.line));
        properties.push(format!("col={}", location.position.column));
    }
    let title = match &report.id {
        Some(id) => format!("zenith({id})"),
        None => "zenith".to_string(),
    };
    properties.push(format!("title={}", escape_property(&title)));

    let mut message = report.title.clone();
    for element in &report.contents {
        if let ReportElement::Note(note) = element
            && note.kind == ReportNoteKind::Help
        {
            message = format!("{message}\n{}", note.message);
        }
    }
    let level = match report.kind {
        ReportKind::Error => "error",
        ReportKind::Warning => "warning",
    };
    format!("::{level} {}::{}", properties.join(","), escape_data(&message))
}

/// `value` escaped for the message of a workflow command.
fn escape_data(value: &str) -> String {
    value.replace('%', "%25").replace('\r', "%0D").replace('\n', "%0A")
}

/// `value` escaped for a property of a workflow command.
fn escape_property(value: &str) -> String {
    escape_data(value).replace(':', "%3A").replace(',', "%2C")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use zenith_diagnostics::Span;
    use zenith_utils::location::ByteRange;

    use super::*;
    use crate::{Outcome, SummaryEntry};

    /// The output of a broken contract `a`, with a report pointing into a
    /// file, in `format`.
    fn render(format: Format) -> String {
        let mut sources = SourceMap::new();
        let file = sources.add("web/a.ts", "import './b';\n");
        let mut report = Report::error("`web` must not import `api`").with_id("a");
        report.add_span(Span::new(file, ByteRange::new(7, 12))).add_help("keep it: down");
        let outcome = Outcome::Broken { findings: 1, kind: ReportKind::Error };
        let entry = SummaryEntry {
            id: "a".into(),
            kind: "forbidden",
            outcome,
            details: Vec::new(),
            duration: Duration::ZERO,
        };
        let summary = Summary {
            entries: vec![entry],
            files: 2,
            imports: 1,
            counts: Vec::new(),
            timings: None,
        };
        Output { sources: &sources, reports: &[report], summary: &summary, format }.to_string()
    }

    #[test]
    fn pretty_shows_the_reports_then_every_contract() {
        let expected = [
            "error[a]: `web` must not import `api`",
            " --> web/a.ts:1:8",
            "  |",
            "1 | import './b';",
            "  |        ^^^^^",
            "  |",
            "  = help: keep it: down",
            "",
            "Contracts",
            "  ✗ a  forbidden  broken (1 violation)",
            "",
            "0 kept, 1 broken · 2 files, 1 import",
        ];
        assert_eq!(render(Format::Pretty), expected.join("\n"));
    }

    #[test]
    fn compact_shows_a_line_per_report() {
        let expected = [
            "web/a.ts:1:8: error[a]: `web` must not import `api`",
            "0 kept, 1 broken · 2 files, 1 import",
        ];
        assert_eq!(render(Format::Compact), expected.join("\n"));
    }

    #[test]
    fn github_shows_a_workflow_command_per_report() {
        let expected = [
            "::error file=web/a.ts,line=1,col=8,title=zenith(a)::`web` must not import `api`%0Akeep it: down",
            "0 kept, 1 broken · 2 files, 1 import",
        ];
        assert_eq!(render(Format::Github), expected.join("\n"));
    }

    #[test]
    fn workflow_commands_are_escaped() {
        assert_eq!(escape_data("50% done\r\nnext"), "50%25 done%0D%0Anext");
        assert_eq!(escape_property("zenith(a,b): c"), "zenith(a%2Cb)%3A c");
    }
}
