//! The [`Parse`] stage.

use std::{iter, mem, time::SystemTime};

use log::{debug, info};
use rayon::prelude::*;
use zenith_parse::{ParsedFile, Parser};

use crate::{
    FileError, FileErrorKind, PipelineError, Session, Stage, StageKind,
    cache::{self, Lookup, Origin, ParseCache},
};

/// Parses the source files with `zenith_parse`, in parallel on the current
/// rayon thread pool. Each worker keeps a parser, so that its arena is reused
/// from file to file. The parser recurses into nested code, so the pool's
/// threads need a deep stack (see [`Parser`]).
///
/// With [`Session::cache`] set, the files that haven't changed since the last
/// run come from the parse cache instead, which is then rewritten if
/// anything changed. Otherwise every file is read and parsed.
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

        // 1. Read the cache, if there is one, and take out the entry of each file.
        let started = SystemTime::now();
        let mut cache = session.cache.as_ref().map(|settings| ParseCache::load(settings, options));
        let entries: Vec<_> = match &mut cache {
            Some(cache) => session.files.iter().map(|file| cache.take(&file.name)).collect(),
            None => iter::repeat_with(|| None).take(session.files.len()).collect(),
        };
        // The entries of files that are gone, which the new cache leaves out.
        let removed = cache.as_ref().map_or(0, ParseCache::len);

        // 2. Parse each file, unless the cache shows that it hasn't changed.
        let results: Vec<Lookup> = session
            .files
            .par_iter()
            .zip(entries)
            .map_init(
                || Parser::with_options(options),
                |parser, (file, entry)| match &cache {
                    Some(cache) => cache.parse(parser, &file.path(root), entry),
                    None => {
                        let parsed = parser.parse_file(&file.path(root));
                        Lookup { parsed, stamp: None, origin: Origin::Parsed }
                    }
                },
            )
            .collect();

        // 3. Move the read errors to the session.
        let mut parsed = Vec::with_capacity(results.len());
        let mut stamps = Vec::with_capacity(results.len());
        let mut unchanged = 0;
        for (file, lookup) in session.files.iter().zip(results) {
            parsed.push(lookup.parsed.unwrap_or_else(|error| {
                let kind = FileErrorKind::Read(error);
                session.file_errors.push(FileError { file: file.name.clone(), kind });
                ParsedFile::default()
            }));
            stamps.push(lookup.stamp);
            session.stats.cached_files += usize::from(lookup.origin != Origin::Parsed);
            unchanged += usize::from(lookup.origin == Origin::Unchanged);
        }

        // 4. Rewrite the cache if anything in it changed: a file was parsed, was read
        //    to find it unchanged, or is gone. A file that can't be read has no entry.
        if let Some(settings) = &session.cache {
            debug!("{unchanged} of {} files are unchanged since the last run", parsed.len());
            let entries: Vec<_> = session
                .files
                .iter()
                .zip(&stamps)
                .zip(&parsed)
                .filter_map(|((file, stamp), parsed)| {
                    Some((file.name.as_str(), stamp.as_ref()?, parsed))
                })
                .collect();
            if (entries.len() != unchanged || removed > 0)
                && let Err(error) = cache::save(settings, options, started, entries.into_iter())
            {
                info!("could not write the parse cache to `{}`: {error}", settings.dir.display());
            }
        }

        // 5. Move the syntax errors to the session.
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
