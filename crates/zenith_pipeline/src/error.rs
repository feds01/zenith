//! What can go wrong in the pipeline: errors that stop it, and problems with
//! single files and imports that don't.

use std::{fmt, io};

use thin_vec::ThinVec;
use zenith_diagnostics::{Report, ReportKind, SourceId, Span};
use zenith_graph::{GraphError, ImportDetail};
use zenith_parse::SyntaxError;
use zenith_resolve::{ResolverError, UnresolvedReason};
use zenith_utils::{location::Located, printing::plural};

/// Why a stage failed, which stops the pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineError {
    /// The resolver could not be created from its settings.
    Resolver(ResolverError),

    /// A file could not be added to the graph.
    Graph(GraphError),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolver(error) => write!(f, "{error}"),
            Self::Graph(error) => write!(f, "{error}"),
        }
    }
}

/// The message is the wrapped error's, so the error has no `source`:
/// reporters that show the chain of causes would show it twice.
impl std::error::Error for PipelineError {}

impl From<ResolverError> for PipelineError {
    fn from(error: ResolverError) -> Self {
        Self::Resolver(error)
    }
}

impl From<GraphError> for PipelineError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

/// A source file that could not be read or parsed in full. The file is still
/// in the graph, with the imports that were found in it, if any.
#[derive(Debug)]
pub struct FileError {
    /// The file, as a POSIX path relative to the root.
    pub file: String,

    /// What is wrong with it.
    pub kind: FileErrorKind,
}

impl FileError {
    /// The report of the error: a warning, since the file stays in the graph.
    /// A syntax error is shown in context when `source`, the file's text in
    /// the source map that the report is rendered with, is given.
    pub fn report(&self, source: Option<SourceId>) -> Report {
        let file = &self.file;
        let errors = match &self.kind {
            FileErrorKind::Read(error) => {
                let mut report = Report::warning(format!("could not read `{file}`: {error}"));
                report.add_note("its imports are missing from the graph");
                return report;
            }
            FileErrorKind::Syntax(errors) => errors,
        };
        let (Some(first), Some(source)) = (errors.first(), source) else {
            let mut report = Report::warning(format!("`{file}` has syntax errors"));
            report.add_note("some of its imports may be missing from the graph");
            return report;
        };

        let mut report = first.value.report(Span::new(source, first.range));
        report.kind = ReportKind::Warning;
        report.add_note("some of the file's imports may be missing from the graph");
        if errors.len() > 1 {
            let count = plural(errors.len(), "syntax error");
            report.add_note(format!("this is the first of {count} in the file"));
        }
        report
    }
}

/// What is wrong with a source file.
#[derive(Debug)]
pub enum FileErrorKind {
    /// It could not be read, or is not UTF-8, so it has no imports.
    Read(io::Error),

    /// It has syntax errors, in source order. If the parser recovered from
    /// them, the imports it found are in the graph.
    Syntax(ThinVec<Located<SyntaxError>>),
}

/// An import whose specifier could not be resolved, so is not in the graph.
#[derive(Debug, Clone, PartialEq)]
pub struct UnresolvedImport {
    /// The importing file, as a POSIX path relative to the root.
    pub file: String,

    /// The import statement or expression.
    pub import: ImportDetail,

    /// Why it could not be resolved.
    pub reason: UnresolvedReason,
}

#[cfg(test)]
mod tests {
    use thin_vec::thin_vec;
    use zenith_diagnostics::{Reporter, SourceMap};
    use zenith_utils::location::ByteRange;

    use super::*;

    /// A syntax error with `message` at `range`.
    fn syntax_error(start: u32, end: u32, message: &'static str) -> Located<SyntaxError> {
        Located::new(ByteRange::new(start, end), SyntaxError { message: message.into() })
    }

    /// The rendered report of a `kind` of error in `a.ts`, whose text is
    /// `text` if it could be read.
    fn render(kind: FileErrorKind, text: Option<&str>) -> String {
        let error = FileError { file: "a.ts".to_string(), kind };
        let mut sources = SourceMap::new();
        let source = text.map(|text| sources.add("a.ts", text));
        Reporter::new(&sources, &[error.report(source)]).to_string()
    }

    #[test]
    fn reports_a_file_that_cannot_be_read() {
        let error = io::Error::new(io::ErrorKind::InvalidData, "not UTF-8");
        let expected = [
            "warning: could not read `a.ts`: not UTF-8",
            "  |",
            "  = note: its imports are missing from the graph",
        ];
        assert_eq!(render(FileErrorKind::Read(error), None), expected.join("\n"));
    }

    #[test]
    fn reports_the_first_syntax_error_in_context() {
        let errors = thin_vec![
            syntax_error(10, 11, "Unexpected token"),
            syntax_error(11, 11, "Expected `;`")
        ];
        let expected = [
            "warning: Unexpected token",
            " --> a.ts:1:11",
            "  |",
            "1 | const a = ;",
            "  |           ^",
            "  |",
            "  = note: some of the file's imports may be missing from the graph",
            "  = note: this is the first of 2 syntax errors in the file",
        ];
        assert_eq!(render(FileErrorKind::Syntax(errors), Some("const a = ;")), expected.join("\n"));
    }

    #[test]
    fn reports_syntax_errors_without_context_when_the_text_is_gone() {
        let errors = thin_vec![syntax_error(8, 9, "Unexpected token")];
        let expected = [
            "warning: `a.ts` has syntax errors",
            "  |",
            "  = note: some of its imports may be missing from the graph",
        ];
        assert_eq!(render(FileErrorKind::Syntax(errors), None), expected.join("\n"));
    }
}
