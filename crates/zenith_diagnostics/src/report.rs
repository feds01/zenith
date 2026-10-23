//! Reports: what is shown to the user about an error or a warning,
//! independently of how it is rendered.

use zenith_utils::location::ByteRange;

use crate::source::{SourceId, Span};

/// How severe a [`Report`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReportKind {
    /// Something that stops zenith from doing what was asked.
    Error,

    /// Something that zenith carries on despite, but that is likely a
    /// mistake.
    Warning,
}

/// The kind of a [`ReportNote`], which labels it when rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReportNoteKind {
    /// How to fix the problem, such as a "did you mean" suggestion.
    Help,

    /// More context about the problem.
    Note,
}

/// A note on a [`Report`] that is not tied to a span of a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportNote {
    /// The kind of the note.
    pub kind: ReportNoteKind,

    /// The message of the note.
    pub message: String,
}

/// An excerpt of a source, with the span that a [`Report`] is about labelled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportCodeBlock {
    /// The span to label.
    pub span: Span,

    /// The label, if the title doesn't say it all.
    pub label: Option<String>,
}

/// A part of a [`Report`], shown in order under its title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportElement {
    /// An excerpt of a source.
    CodeBlock(ReportCodeBlock),

    /// A note.
    Note(ReportNote),
}

/// What is shown to the user about an error or a warning: a title, followed
/// by excerpts of the sources that it is about, and notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// How severe the report is.
    pub kind: ReportKind,

    /// What is wrong, in a line.
    pub title: String,

    /// What the report is about, shown next to its kind, e.g. the contract in
    /// `error[web-layers]: ...`.
    pub id: Option<String>,

    /// The excerpts and notes, in the order that they are shown.
    pub contents: Vec<ReportElement>,

    /// The source that the byte ranges added to the report are in, if it is
    /// known.
    pub source: Option<SourceId>,
}

/// What an excerpt of a [`Report`] marks: a [`Span`], or a [`ByteRange`] of
/// the report's source.
pub trait ReportSpan {
    /// The span that it is in a report whose source is `source`. A range has
    /// none when the source is not known.
    fn span_in(self, source: Option<SourceId>) -> Option<Span>;
}

impl ReportSpan for Span {
    fn span_in(self, _: Option<SourceId>) -> Option<Span> {
        Some(self)
    }
}

impl ReportSpan for ByteRange {
    fn span_in(self, source: Option<SourceId>) -> Option<Span> {
        source.map(|source| Span::new(source, self))
    }
}

impl Report {
    /// A report with a title, and no excerpts or notes yet.
    pub fn new(kind: ReportKind, title: impl Into<String>) -> Self {
        Self { kind, title: title.into(), id: None, contents: Vec::new(), source: None }
    }

    /// An error report.
    pub fn error(title: impl Into<String>) -> Self {
        Self::new(ReportKind::Error, title)
    }

    /// A warning report.
    pub fn warning(title: impl Into<String>) -> Self {
        Self::new(ReportKind::Warning, title)
    }

    /// Set what the report is about, e.g. a contract id.
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Whether the report is of an error.
    pub fn is_error(&self) -> bool {
        self.kind == ReportKind::Error
    }

    /// Add an excerpt that marks `span` without a label. A byte range is in
    /// the report's source, and is left out when that is not known.
    pub fn add_span(&mut self, span: impl ReportSpan) -> &mut Self {
        self.add_code_block(span, None)
    }

    /// Add an excerpt that labels `span`, as [`add_span`](Self::add_span)
    /// does.
    pub fn add_labelled_span(
        &mut self,
        span: impl ReportSpan,
        label: impl Into<String>,
    ) -> &mut Self {
        self.add_code_block(span, Some(label.into()))
    }

    /// Add an excerpt that marks `span`, with a label if any.
    fn add_code_block(&mut self, span: impl ReportSpan, label: Option<String>) -> &mut Self {
        match span.span_in(self.source) {
            Some(span) => {
                self.add_element(ReportElement::CodeBlock(ReportCodeBlock { span, label }))
            }
            None => self,
        }
    }

    /// Add a note on how to fix the problem.
    pub fn add_help(&mut self, message: impl Into<String>) -> &mut Self {
        self.add_note_of(ReportNoteKind::Help, message)
    }

    /// Add a note with more context about the problem.
    pub fn add_note(&mut self, message: impl Into<String>) -> &mut Self {
        self.add_note_of(ReportNoteKind::Note, message)
    }

    /// Add a note of the given kind.
    fn add_note_of(&mut self, kind: ReportNoteKind, message: impl Into<String>) -> &mut Self {
        self.add_element(ReportElement::Note(ReportNote { kind, message: message.into() }))
    }

    /// Add an excerpt or a note.
    pub fn add_element(&mut self, element: ReportElement) -> &mut Self {
        self.contents.push(element);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    #[test]
    fn keeps_its_elements_in_the_order_they_are_added() {
        let mut sources = SourceMap::new();
        let span = Span::new(sources.add("zenith.toml", "[files]\n"), ByteRange::new(0, 7));

        let mut report = Report::error("unknown table");
        report.add_note("in the root table").add_labelled_span(span, "here").add_help("remove it");
        assert!(report.is_error());
        assert_eq!(
            report.contents,
            [
                ReportElement::Note(ReportNote {
                    kind: ReportNoteKind::Note,
                    message: "in the root table".into()
                }),
                ReportElement::CodeBlock(ReportCodeBlock { span, label: Some("here".into()) }),
                ReportElement::Note(ReportNote {
                    kind: ReportNoteKind::Help,
                    message: "remove it".into()
                }),
            ]
        );
        assert!(!Report::warning("unused entry").is_error());
    }

    #[test]
    fn marks_ranges_in_its_source_when_it_is_known() {
        let mut sources = SourceMap::new();
        let source = sources.add("zenith.toml", "[files]\n");
        let range = ByteRange::new(1, 6);

        let mut report = Report::error("unknown table");
        report.add_span(range);
        assert_eq!(report.contents, []);
        report.source = Some(source);
        report.add_labelled_span(range, "here");
        let span = Span::new(source, range);
        assert_eq!(
            report.contents,
            [ReportElement::CodeBlock(ReportCodeBlock { span, label: Some("here".into()) })]
        );
    }
}
