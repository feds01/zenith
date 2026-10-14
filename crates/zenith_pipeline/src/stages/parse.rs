//! The [`Parse`] stage.

use std::{io, mem};

use rayon::prelude::*;
use zenith_parse::{ParsedFile, Parser};

use crate::{FileError, FileErrorKind, PipelineError, Session, Stage, StageKind};

/// Parses the source files with `zenith_parse`, in parallel on the current
/// rayon thread pool. Each worker keeps a parser, so that its arena is reused
/// from file to file. The parser recurses into nested code, so the pool's
/// threads need a deep stack (see [`Parser`]).
///
/// A file that can't be read parses as empty. Read and syntax errors are
/// moved to the session's file errors.
pub struct Parse;

impl Stage for Parse {
    fn kind(&self) -> StageKind {
        StageKind::Parse
    }

    fn run(&self, session: &mut Session) -> Result<(), PipelineError> {
        let options = session.parse_options();
        let root = &session.root;
        let results: Vec<io::Result<ParsedFile>> = session
            .files
            .par_iter()
            .map_init(
                || Parser::with_options(options),
                |parser, file| parser.parse_file(&file.path(root)),
            )
            .collect();

        let mut parsed = Vec::with_capacity(results.len());
        for (file, result) in session.files.iter().zip(results) {
            let mut file_parsed = result.unwrap_or_else(|error| {
                let kind = FileErrorKind::Read(error);
                session.file_errors.push(FileError { file: file.name.clone(), kind });
                ParsedFile::default()
            });
            if !file_parsed.errors.is_empty() {
                let kind = FileErrorKind::Syntax(mem::take(&mut file_parsed.errors));
                session.file_errors.push(FileError { file: file.name.clone(), kind });
            }
            parsed.push(file_parsed);
        }

        // The files are sorted by path, which isn't quite the order of names.
        session.file_errors.sort_by(|a, b| a.file.cmp(&b.file));
        session.parsed = parsed;
        Ok(())
    }
}
