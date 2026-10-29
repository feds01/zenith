//! Rendering reports with `annotate-snippets`, or as GitHub Actions workflow
//! commands.

use std::{fmt, ops::Range};

use annotate_snippets::{Annotation, AnnotationKind, Element, Group, Level, Renderer, Snippet};
use rayon::prelude::*;

use crate::{
    report::{Report, ReportCodeBlock, ReportElement, ReportKind, ReportNoteKind},
    source::SourceMap,
};

/// Renders reports, reading their excerpts from a [`SourceMap`]. Each report
/// is rendered on its own, and a blank line separates them, unless they are
/// rendered [short](Reporter::short) or as [workflow
/// commands](Reporter::github), a line each.
pub struct Reporter<'a> {
    /// The sources that the reports point into.
    sources: &'a SourceMap,

    /// The reports to render.
    reports: &'a [Report],

    /// How to render them in full or short: plain by default.
    renderer: Renderer,

    /// How much of each report to render, and how.
    style: Style,
}

/// How a [`Reporter`] renders each report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    /// With its excerpts and notes.
    Full,

    /// As a line, without its excerpts and notes.
    Short,

    /// As a GitHub Actions workflow command.
    Github,
}

impl<'a> Reporter<'a> {
    /// A reporter that renders `reports` without colour.
    pub fn new(sources: &'a SourceMap, reports: &'a [Report]) -> Self {
        Self { sources, reports, renderer: Renderer::plain(), style: Style::Full }
    }

    /// Render with `renderer` instead, e.g. [`Renderer::styled`] for a
    /// terminal.
    pub fn with_renderer(self, renderer: Renderer) -> Self {
        Self { renderer, ..self }
    }

    /// Render in colour if `colour` is set, and plainly otherwise.
    pub fn with_colour(self, colour: bool) -> Self {
        self.with_renderer(if colour { Renderer::styled() } else { Renderer::plain() })
    }

    /// Render each report as a line, `path:line:col: level[id]: title`, where
    /// its first excerpt starts, as `rustc --error-format=short` does. A
    /// report without excerpts is `level[id]: title`.
    pub fn short(self) -> Self {
        Self { style: Style::Short, ..self }
    }

    /// Render each report as a GitHub Actions workflow command, which shows
    /// it as an annotation on its line of a pull request: `::level
    /// file=…,line=…,col=…,title=zenith(id)::title`, where its first excerpt
    /// starts, with its help after its title.
    pub fn github(self) -> Self {
        Self { style: Style::Github, ..self }
    }

    /// The `annotate-snippets` group of a report.
    fn group(&self, report: &'a Report) -> Group<'a> {
        let level = match report.kind {
            ReportKind::Error => Level::ERROR,
            ReportKind::Warning => Level::WARNING,
        };
        let elements = report.contents.iter().map(|element| -> Element<'a> {
            match element {
                ReportElement::CodeBlock(block) => self.snippet(block).into(),
                ReportElement::Note(note) => {
                    let level = match note.kind {
                        ReportNoteKind::Help => Level::HELP,
                        ReportNoteKind::Note => Level::NOTE,
                    };
                    level.message(&note.message).into()
                }
            }
        });
        let title = level.primary_title(&report.title);
        let title = match &report.id {
            Some(id) => title.id(id),
            None => title,
        };
        title.elements(elements)
    }

    /// The excerpt of a code block, with its span labelled. Only the lines of
    /// the span are given to `annotate-snippets`, which reads every line of
    /// the source it is given.
    fn snippet(&self, block: &'a ReportCodeBlock) -> Snippet<'a, Annotation<'a>> {
        let source = self.sources.get(block.span.source);
        let (line, lines) = source.lines(block.span.range);
        let start = (block.span.range.start() - lines.start()) as usize;
        let end = (block.span.range.end() - lines.start()) as usize;
        let annotation = AnnotationKind::Primary.span(start..end).label(block.label.as_deref());
        Snippet::source(&source.contents[Range::from(lines)])
            .line_start(line as usize)
            .path(&source.path)
            .fold(true)
            .annotation(annotation)
    }

    /// The workflow command of a report.
    fn workflow_command(&self, report: &Report) -> String {
        let mut properties = Vec::new();
        let span = report.contents.iter().find_map(|element| match element {
            ReportElement::CodeBlock(block) => Some(block.span),
            ReportElement::Note(_) => None,
        });
        if let Some(span) = span {
            let location = self.sources.location(span);
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
}

impl fmt::Display for Reporter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Each report is rendered apart from the others, so they are rendered in
        // parallel, and shown in order.
        let renderer = self.renderer.clone().short_message(self.style == Style::Short);
        let separator = if self.style == Style::Full { "\n\n" } else { "\n" };
        let rendered: Vec<String> = (self.reports.par_iter())
            .map(|report| match self.style {
                Style::Full | Style::Short => renderer.render(&[self.group(report)]),
                Style::Github => self.workflow_command(report),
            })
            .collect();
        for (index, report) in rendered.iter().enumerate() {
            if index > 0 {
                f.write_str(separator)?;
            }
            f.write_str(report)?;
        }
        Ok(())
    }
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
    use zenith_utils::location::ByteRange;

    use super::*;
    use crate::source::Span;

    /// The configuration file that the tests report on.
    const CONFIG: &str = "[files]\nexclud = []\n\n[contracts.a]\ntype = \"layer\"\n";

    #[test]
    fn renders_excerpts_and_notes() {
        let mut sources = SourceMap::new();
        let config = sources.add("zenith.toml", CONFIG);

        let mut unknown_key = Report::error("unknown key `exclud`");
        unknown_key
            .add_span(Span::new(config, ByteRange::new(8, 14)))
            .add_help("did you mean `exclude`?");
        let mut unknown_type = Report::warning("unknown contract type `layer`");
        unknown_type
            .add_labelled_span(Span::new(config, ByteRange::new(42, 49)), "in contract `a`")
            .add_note("contracts of an unknown type are skipped");
        let reports = [unknown_key, unknown_type];

        let rendered = Reporter::new(&sources, &reports).to_string();
        let expected = [
            "error: unknown key `exclud`",
            " --> zenith.toml:2:1",
            "  |",
            "2 | exclud = []",
            "  | ^^^^^^",
            "  |",
            "  = help: did you mean `exclude`?",
            "",
            "warning: unknown contract type `layer`",
            " --> zenith.toml:5:8",
            "  |",
            "5 | type = \"layer\"",
            "  |        ^^^^^^^ in contract `a`",
            "  |",
            "  = note: contracts of an unknown type are skipped",
        ];
        assert_eq!(rendered, expected.join("\n"));
    }

    #[test]
    fn renders_the_id_next_to_the_kind() {
        let sources = SourceMap::new();
        let mut report = Report::warning("`web` must not import `api`").with_id("web-layers");
        report.add_help("move the shared code down a layer");
        let expected = [
            "warning[web-layers]: `web` must not import `api`",
            "  |",
            "  = help: move the shared code down a layer",
        ];
        assert_eq!(Reporter::new(&sources, &[report]).to_string(), expected.join("\n"));
    }

    #[test]
    fn renders_an_empty_span_at_the_end_of_a_source() {
        let mut sources = SourceMap::new();
        let config = sources.add("zenith.toml", "[files]\nexclude = [\n");
        let mut report = Report::error("invalid TOML: unclosed array");
        report.add_span(Span::new(config, ByteRange::new(20, 20)));

        let expected = [
            "error: invalid TOML: unclosed array",
            " --> zenith.toml:2:13",
            "  |",
            "2 | exclude = [",
            "  |            ^",
        ];
        assert_eq!(Reporter::new(&sources, &[report]).to_string(), expected.join("\n"));
    }

    #[test]
    fn renders_short_reports_a_line_each() {
        let mut sources = SourceMap::new();
        let config = sources.add("zenith.toml", CONFIG);
        let mut unknown_key = Report::error("unknown key `exclud`").with_id("a");
        unknown_key
            .add_labelled_span(Span::new(config, ByteRange::new(8, 14)), "here")
            .add_help("did you mean `exclude`?");
        let reports = [unknown_key, Report::warning("no contracts")];

        let expected =
            ["zenith.toml:2:1: error[a]: unknown key `exclud`: here", "warning: no contracts"];
        assert_eq!(Reporter::new(&sources, &reports).short().to_string(), expected.join("\n"));
    }

    #[test]
    fn renders_workflow_commands_a_line_each() {
        let mut sources = SourceMap::new();
        let file = sources.add("web/a.ts", "import './b';\n");
        let mut report = Report::error("`web` must not import `api`").with_id("a");
        report.add_span(Span::new(file, ByteRange::new(7, 12))).add_help("keep it: down");
        let reports = [report, Report::warning("no contracts")];

        let expected = [
            "::error file=web/a.ts,line=1,col=8,title=zenith(a)::`web` must not import `api`%0Akeep it: down",
            "::warning title=zenith::no contracts",
        ];
        assert_eq!(Reporter::new(&sources, &reports).github().to_string(), expected.join("\n"));
    }

    #[test]
    fn workflow_commands_are_escaped() {
        assert_eq!(escape_data("50% done\r\nnext"), "50%25 done%0D%0Anext");
        assert_eq!(escape_property("zenith(a,b): c"), "zenith(a%2Cb)%3A c");
    }

    #[test]
    fn renders_nothing_without_reports() {
        assert_eq!(Reporter::new(&SourceMap::new(), &[]).to_string(), "");
    }
}
