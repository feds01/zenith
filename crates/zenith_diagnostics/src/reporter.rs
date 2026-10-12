//! Rendering reports with `annotate-snippets`.

use std::{fmt, ops::Range};

use annotate_snippets::{Annotation, AnnotationKind, Element, Group, Level, Renderer, Snippet};

use crate::{
    report::{Report, ReportCodeBlock, ReportElement, ReportKind, ReportNoteKind},
    source::SourceMap,
};

/// Renders reports, reading their excerpts from a [`SourceMap`]. Each report
/// is rendered on its own, and a blank line separates them.
pub struct Reporter<'a> {
    /// The sources that the reports point into.
    sources: &'a SourceMap,

    /// The reports to render.
    reports: &'a [Report],

    /// How to render them: plain by default.
    renderer: Renderer,
}

impl<'a> Reporter<'a> {
    /// A reporter that renders `reports` without colour.
    pub fn new(sources: &'a SourceMap, reports: &'a [Report]) -> Self {
        Self { sources, reports, renderer: Renderer::plain() }
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

    /// The excerpt of a code block, with its span labelled.
    fn snippet(&self, block: &'a ReportCodeBlock) -> Snippet<'a, Annotation<'a>> {
        let source = self.sources.get(block.span.source);
        let annotation = AnnotationKind::Primary
            .span(Range::from(block.span.range))
            .label(block.label.as_deref());
        Snippet::source(&source.contents).path(&source.path).fold(true).annotation(annotation)
    }
}

impl fmt::Display for Reporter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, report) in self.reports.iter().enumerate() {
            if index > 0 {
                f.write_str("\n\n")?;
            }
            f.write_str(&self.renderer.render(&[self.group(report)]))?;
        }
        Ok(())
    }
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
    fn renders_nothing_without_reports() {
        assert_eq!(Reporter::new(&SourceMap::new(), &[]).to_string(), "");
    }
}
