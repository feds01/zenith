//! The problems that the pipeline finds, as the JSON output of `zenith check`
//! shows them: the imports that couldn't be resolved, and the errors in
//! source files that couldn't be read or parsed in full.

use serde::Serialize;
use zenith_diagnostics::SourceMap;

use crate::{FileError, FileErrorKind, UnresolvedImport};

/// An import that couldn't be resolved.
#[derive(Debug, Serialize)]
pub struct UnresolvedJson<'a> {
    /// The importing file.
    file: &'a str,

    /// The 1-based line of the import, if the file can be read.
    line: Option<u32>,

    /// The 1-based column of the import, if the file can be read.
    column: Option<u32>,

    /// The specifier, as written.
    specifier: &'a str,

    /// Why it couldn't be resolved.
    reason: String,
}

/// An error in a source file that couldn't be read or parsed in full.
#[derive(Debug, Serialize)]
pub struct FileProblemJson<'a> {
    /// The file.
    file: &'a str,

    /// The 1-based line of a syntax error.
    line: Option<u32>,

    /// The 1-based column of a syntax error.
    column: Option<u32>,

    /// What is wrong.
    message: String,
}

impl UnresolvedImport {
    /// The import as JSON, with where it is in `sources`.
    pub fn json(&self, sources: &mut SourceMap) -> UnresolvedJson<'_> {
        let (line, column) = position(sources, &self.file, self.import.range.start());
        UnresolvedJson {
            file: &self.file,
            line,
            column,
            specifier: &self.import.specifier,
            reason: self.reason.to_string(),
        }
    }
}

impl FileError {
    /// The problems of the file as JSON: one per syntax error, with where it
    /// is in `sources`, or the error that it couldn't be read with.
    pub fn json(&self, sources: &mut SourceMap) -> Vec<FileProblemJson<'_>> {
        let file = &self.file;
        match &self.kind {
            FileErrorKind::Read(error) => {
                vec![FileProblemJson { file, line: None, column: None, message: error.to_string() }]
            }
            FileErrorKind::Syntax(errors) => errors
                .iter()
                .map(|syntax| {
                    let (line, column) = position(sources, file, syntax.range.start());
                    FileProblemJson { file, line, column, message: syntax.value.to_string() }
                })
                .collect(),
        }
    }
}

/// The line and column of the byte `offset` of `file` in `sources`, if it can
/// be read.
fn position(sources: &mut SourceMap, file: &str, offset: u32) -> (Option<u32>, Option<u32>) {
    match sources.file(file) {
        Some(id) => {
            let position = sources.get(id).position(offset);
            (Some(position.line), Some(position.column))
        }
        None => (None, None),
    }
}
