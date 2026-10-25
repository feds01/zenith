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
/// anything changed. A session that doesn't read the disk, such as one of a
/// commit, only reads the cache, so that it can't replace the entries of the
/// files on the disk. Otherwise every file is read from the session's file
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
            None => {
                let results = parse_files(session, &session.files);
                read_all(&mut session.file_errors, &session.files, results)
            }
        };

        // 2. Move the syntax errors to the session.
        move_syntax_errors(&mut session.file_errors, &session.files, &mut parsed);
        session.parsed = parsed;
        Ok(())
    }

    /// Reads and parses the changed files again, in place of what they parsed
    /// to before, with their read and syntax errors in place of theirs. The
    /// cache stays as it is, so that the next run finds them changed.
    fn update(&self, session: &mut Session, changed: &[u32]) -> Result<(), PipelineError> {
        let files: Vec<_> =
            changed.iter().map(|&index| session.files[index as usize].clone()).collect();
        session.file_errors.retain(|error| files.iter().all(|file| file.name != error.file));
        let results = parse_files(session, &files);
        let mut parsed = read_all(&mut session.file_errors, &files, results);
        move_syntax_errors(&mut session.file_errors, &files, &mut parsed);
        for (&index, parsed) in changed.iter().zip(parsed) {
            session.parsed[index as usize] = parsed;
        }
        Ok(())
    }
}

/// Read and parse `files`, of `session`, from its file system.
fn parse_files(session: &Session, files: &[SourceFile]) -> Vec<io::Result<ParsedFile>> {
    let (root, file_system) = (&session.root, &*session.file_system);
    files
        .par_iter()
        .map_init(
            || session.backends.parsers(),
            |parsers, file| {
                let path = file.path(root);
                let source = file_system.read_text(&path)?;
                Ok(parsers[usize::from(file.language)].parse_source(&path, &source))
            },
        )
        .collect()
}

/// What each of `files` parsed to, as `results` has it, or an empty file for
/// each that couldn't be read, with the error moved to `file_errors`.
fn read_all(
    file_errors: &mut Vec<FileError>,
    files: &[SourceFile],
    results: Vec<io::Result<ParsedFile>>,
) -> Vec<ParsedFile> {
    let files = files.iter().zip(results);
    files.map(|(file, result)| read_or_default(file_errors, file, result)).collect()
}

/// Move the syntax errors of what each of `files` parsed to, in `parsed`, to
/// `file_errors`, and sort them by file.
fn move_syntax_errors(
    file_errors: &mut Vec<FileError>,
    files: &[SourceFile],
    parsed: &mut [ParsedFile],
) {
    for (file, parsed) in files.iter().zip(parsed) {
        if !parsed.errors.is_empty() {
            let kind = FileErrorKind::Syntax(mem::take(&mut parsed.errors));
            file_errors.push(FileError { file: file.name.clone(), kind });
        }
    }
    // The files are sorted by path, which isn't quite the order of names.
    file_errors.sort_by(|a, b| a.file.cmp(&b.file));
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
    let (root, file_system) = (&session.root, &*session.file_system);
    let results: Vec<Lookup<ParsedFile>> = session
        .files
        .par_iter()
        .zip(entries)
        .map_init(
            || session.backends.parsers(),
            |parsers, (file, entry)| {
                let path = file.path(root);
                let parser = &mut parsers[usize::from(file.language)];
                cache.lookup(file_system, &path, entry, |source| parser.parse_source(&path, source))
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

    // 4. Rewrite the cache if anything in it changed, and the files are the disk's:
    //    a file was parsed, was read to find it unchanged, or is gone. A file that
    //    can't be read has no entry.
    debug!("{}", message!(pipeline_unchanged_files, unchanged, total = parsed.len()));
    let entries: Vec<_> = session
        .files
        .iter()
        .zip(&stamps)
        .zip(&parsed)
        .filter_map(|((file, stamp), parsed)| Some((file.name.as_str(), stamp.as_ref()?, parsed)))
        .collect();
    if session.file_system.on_disk()
        && (entries.len() != unchanged || removed > 0)
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
