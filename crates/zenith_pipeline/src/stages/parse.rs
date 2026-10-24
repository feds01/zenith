//! The [`Parse`] stage.

use std::{io, mem, time::SystemTime};

use log::{debug, info};
use rayon::prelude::*;
use zenith_cache::{Cache, Lookup, Origin};
use zenith_diagnostics::message;
use zenith_lang::ParsedFile;

use crate::{FileError, FileErrorKind, PipelineError, Session, SourceFile, Stage, StageKind};

/// Parses the source files with the backend of each one's language, in
/// parallel on the current rayon thread pool. Each worker keeps a parser of
/// each language, so that their buffers are reused from file to file. Parsers
/// may recurse into nested code, so the pool's threads need a deep stack.
///
/// With [`Session::cache`] set, the files that haven't changed since the last
/// run come from the parse cache instead, which is then rewritten if
/// anything changed. Otherwise every file is read from the session's file
/// system and parsed.
///
/// A file that can't be read parses as empty. Read and syntax errors are
/// moved to the session's file errors.
pub struct Parse;

impl Stage for Parse {
    fn kind(&self) -> StageKind {
        StageKind::Parse
    }

    fn run(&self, session: &mut Session) -> Result<(), PipelineError> {
        // 1. Parse every file, or take the unchanged ones from the cache, which is only
        //    valid for the same languages, recording the same things.
        let mut parsed = match session.cache.clone() {
            Some(settings) => {
                let languages: Vec<_> =
                    session.config.languages.enabled().map(|language| language.name).collect();
                let inputs = (languages, session.needs_graph_through_reexports());
                parse_cached(session, Cache::load(settings, inputs))
            }
            None => parse_all(session),
        };

        // 2. Move the syntax errors to the session.
        for (file, parsed) in session.files.iter().zip(&mut parsed) {
            if !parsed.errors.is_empty() {
                let kind = FileErrorKind::Syntax(mem::take(&mut parsed.errors));
                session.file_errors.push(FileError { file: file.name.clone(), kind });
            }
        }

        // The files are sorted by path, which isn't quite the order of names.
        session.file_errors.sort_by(|a, b| a.file.cmp(&b.file));
        session.parsed = parsed;
        Ok(())
    }
}

/// Read and parse every file of `session`, moving the read errors to the
/// session.
fn parse_all(session: &mut Session) -> Vec<ParsedFile> {
    let (root, file_system) = (&session.root, &*session.file_system);
    let results: Vec<io::Result<ParsedFile>> = session
        .files
        .par_iter()
        .map_init(
            || session.backends.parsers(),
            |parsers, file| {
                let path = file.path(root);
                let source = file_system.read_text(&path)?;
                Ok(parsers[usize::from(file.language)].parse_source(&path, &source))
            },
        )
        .collect();
    let files = session.files.iter().zip(results);
    files.map(|(file, result)| read_or_default(&mut session.file_errors, file, result)).collect()
}

/// Parse the files of `session` that `cache` doesn't show unchanged, moving
/// the read errors to the session, then rewrite the cache if anything in it
/// changed.
fn parse_cached(session: &mut Session, mut cache: Cache<ParsedFile>) -> Vec<ParsedFile> {
    // 1. Take out the entry of each file. Those left are of files that are gone.
    let started = SystemTime::now();
    let entries: Vec<_> = session.files.iter().map(|file| cache.take(&file.name)).collect();
    let removed = cache.len();

    // 2. Parse each file, unless its entry shows that it hasn't changed.
    let root = &session.root;
    let results: Vec<Lookup<ParsedFile>> = session
        .files
        .par_iter()
        .zip(entries)
        .map_init(
            || session.backends.parsers(),
            |parsers, (file, entry)| {
                let path = file.path(root);
                let parser = &mut parsers[usize::from(file.language)];
                cache.lookup(&path, entry, |source| parser.parse_source(&path, source))
            },
        )
        .collect();

    // 3. Move the read errors to the session.
    let mut parsed = Vec::with_capacity(results.len());
    let mut stamps = Vec::with_capacity(results.len());
    let mut unchanged = 0;
    for (file, lookup) in session.files.iter().zip(results) {
        parsed.push(read_or_default(&mut session.file_errors, file, lookup.value));
        stamps.push(lookup.stamp);
        session.stats.cached_files += usize::from(lookup.origin != Origin::Computed);
        unchanged += usize::from(lookup.origin == Origin::Unchanged);
    }

    // 4. Rewrite the cache if anything in it changed: a file was parsed, was read
    //    to find it unchanged, or is gone. A file that can't be read has no entry.
    debug!("{}", message!(pipeline_unchanged_files, unchanged, total = parsed.len()));
    let entries: Vec<_> = session
        .files
        .iter()
        .zip(&stamps)
        .zip(&parsed)
        .filter_map(|((file, stamp), parsed)| Some((file.name.as_str(), stamp.as_ref()?, parsed)))
        .collect();
    if (entries.len() != unchanged || removed > 0)
        && let Err(error) = cache.save(started, entries.into_iter())
    {
        let dir = &cache.settings().dir;
        info!("{}", message!(pipeline_cache_not_written, dir = dir.display(), error));
    }
    parsed
}

/// What `file` parsed to, or an empty file if it couldn't be read, with the
/// error moved to `file_errors`.
fn read_or_default(
    file_errors: &mut Vec<FileError>,
    file: &SourceFile,
    result: io::Result<ParsedFile>,
) -> ParsedFile {
    result.unwrap_or_else(|error| {
        file_errors.push(FileError { file: file.name.clone(), kind: FileErrorKind::Read(error) });
        ParsedFile::default()
    })
}
