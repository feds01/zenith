//! The [`Resolve`] stage, and what it keeps in the cache.

use std::{
    hash::{Hash, Hasher},
    path::Path,
};

use log::info;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use zenith_cache::{Cached, Decoder, Encoder, Observed, Snapshot, stable_hash};
use zenith_diagnostics::message;
use zenith_lang::{ImportResolver, ParsedFile, Resolved};

use crate::{PipelineError, ResolvedImport, Session, SourceFile, Stage, StageKind};

/// Resolves the specifier of every parsed import with the backend of its
/// file's language, in parallel, sharing one resolver of each language and its
/// caches between the workers.
///
/// With [`Session::cache`] set, what the imports of each file resolved to is
/// kept in the cache, with what the backends read to find the packages and
/// to resolve them, and taken from it for the files whose imports are
/// unchanged while nothing of that has changed: see [`Snapshot`]. Only the
/// languages whose imports [resolve by their
/// specifier](zenith_lang::Backend::resolves_by_specifier) keep them.
pub struct Resolve;

impl Stage for Resolve {
    fn kind(&self) -> StageKind {
        StageKind::Resolve
    }

    fn run(&self, session: &mut Session) -> Result<(), PipelineError> {
        // 1. Take what the imports of each file resolved to last time, if nothing that
        //    the backends read then has changed since.
        let snapshot = match &session.cache {
            Some(cache) if session.file_system.on_disk() => {
                Some(Snapshot::<Resolutions>::new(cache.clone(), session.resolution_inputs()))
            }
            _ => None,
        };
        let (kept, before) = match snapshot.as_ref().and_then(Snapshot::load_observed) {
            Some((kept, before)) => (Some(kept), Some(before)),
            None => (None, None),
        };

        // 2. Resolve the imports of the other files, recording what the backends read
        //    to resolve them if they are to be kept.
        let (resolutions, read) = match &snapshot {
            Some(_) => {
                let (resolutions, read) = session.recorder.record(|| resolve(session, kept));
                (resolutions?, Some(read))
            }
            None => (resolve(session, kept)?, None),
        };
        session.stats.cached_resolutions = resolutions.kept;

        // 3. Keep them, with what the backends read, if any were resolved again.
        if let (Some(snapshot), Some(mut read), Some((started, discovery)), Some(cache)) =
            (&snapshot, read, &session.read_by_discovery, &session.cache)
            && resolutions.resolved_again
        {
            // What was seen where no directory tells is checked as what was there
            // before.
            read.merge(discovery);
            let before = before.unwrap_or_else(|| snapshot.observe(&[], &[]));
            let before = before.and(Observed::seen(read.kinds));
            let saved = snapshot.save(*started, &resolutions.value, &before, &read.contents, &[]);
            if let Err(error) = saved {
                let dir = cache.dir.display();
                info!("{}", message!(pipeline_resolutions_not_saved, dir, error));
            }
        }

        session.resolved = resolutions.value.resolved;
        Ok(())
    }
}

impl Session {
    /// The hash of what resolving the imports depends on besides what the
    /// backends read: the root, the files and the manifests, and the options
    /// of each language.
    fn resolution_inputs(&self) -> u64 {
        // Every language's options are plain data, all of which their `Debug`
        // shows.
        let languages: Vec<String> = (self.config.languages.iter())
            .map(|(language, options)| format!("{} {options:?}", language.name))
            .collect();
        let reexports = self.needs_graph_through_reexports();
        stable_hash((&self.root, &self.files, &self.manifests, languages, reexports))
    }
}

/// What the imports of the session's files resolved to, and whether the
/// cache kept them.
struct Resolving {
    /// What they resolved to.
    value: Resolutions,

    /// The files whose imports the cache kept.
    kept: usize,

    /// Whether the imports of a file of a language that keeps them were
    /// resolved again, so that what the cache keeps is out of date.
    resolved_again: bool,
}

/// Resolve the imports of the session's files, taking those of each file
/// whose imports are unchanged from what the cache `kept`.
fn resolve(session: &Session, kept: Option<Resolutions>) -> Result<Resolving, PipelineError> {
    // 1. Tell which files' imports were resolved last time: those of a language
    //    that keeps them, whose imports are the same.
    let backends = &session.backends;
    let imports: Vec<Option<u64>> = (session.files.par_iter())
        .zip(&session.parsed)
        .map(|(file, parsed)| {
            backends.of(file).resolves_by_specifier().then(|| imports_hash(parsed))
        })
        .collect();
    let (mut previous, kept_imports) = match kept {
        Some(kept) => (kept.resolved, kept.imports),
        None => (Vec::new(), Vec::new()),
    };
    let unchanged: Vec<bool> = (imports.iter().enumerate())
        .map(|(index, imports)| imports.is_some() && kept_imports.get(index) == Some(imports))
        .collect();
    previous.resize_with(session.files.len(), Vec::new);

    // 2. Resolve the others' imports, with resolvers made only if there are any:
    //    first those of the languages that keep them, recording what is read.
    let kept = unchanged.iter().filter(|unchanged| **unchanged).count();
    let resolvers = match kept < session.files.len() {
        true => Some(session.backends.resolvers()?),
        false => None,
    };
    let root = &session.root;
    // Most imports resolve to a source file, which is kept by its index, so that
    // their names don't pile up until the graph is built.
    let indices: FxHashMap<&str, u32> =
        (0..).zip(&session.files).map(|(index, file)| (file.name.as_str(), index)).collect();
    let resolve = |file: &SourceFile, parsed: &ParsedFile| match &resolvers {
        Some(resolvers) => {
            let resolver = &*resolvers[usize::from(file.language)];
            resolve_file(resolver, root, file, parsed, &indices)
        }
        None => Vec::new(),
    };
    let mut resolved: Vec<_> = (session.files.par_iter())
        .zip(&session.parsed)
        .zip(previous.into_par_iter())
        .zip(&unchanged)
        .zip(&imports)
        .map(|((((file, parsed), previous), unchanged), imports)| match (unchanged, imports) {
            (true, _) => previous,
            (false, Some(_)) => resolve(file, parsed),
            (false, None) => Vec::new(),
        })
        .collect();

    // 3. Then those of the other languages, which nothing that is kept depends on.
    session.recorder.unrecorded(|| {
        (resolved.par_iter_mut())
            .zip(&session.files)
            .zip(&session.parsed)
            .zip(&imports)
            .filter(|(_, imports)| imports.is_none())
            .for_each(|(((resolved, file), parsed), _)| *resolved = resolve(file, parsed));
    });

    let resolved_again = (imports.iter().zip(&unchanged))
        .any(|(imports, unchanged)| imports.is_some() && !unchanged);
    let value = Resolutions { imports, resolved };
    Ok(Resolving { value, kept, resolved_again })
}

/// What the imports of `file`, whose parser found `parsed`, resolve to with
/// `resolver`.
fn resolve_file(
    resolver: &dyn ImportResolver,
    root: &Path,
    file: &SourceFile,
    parsed: &ParsedFile,
    indices: &FxHashMap<&str, u32>,
) -> Vec<ResolvedImport> {
    let path = file.path(root);
    (parsed.imports.iter())
        .map(|import| match resolver.resolve_import(&path, import) {
            Resolved::Internal(name) => match indices.get(name.as_str()) {
                Some(index) => ResolvedImport::File(*index),
                None => ResolvedImport::Other(Resolved::Internal(name)),
            },
            other => ResolvedImport::Other(other),
        })
        .collect()
}

/// The hash of what the imports of a file whose parser found `parsed`
/// resolve by: their specifiers, in order.
fn imports_hash(parsed: &ParsedFile) -> u64 {
    stable_hash(Specifiers(parsed))
}

/// The specifiers of the imports that a file's parser found, to hash.
struct Specifiers<'a>(&'a ParsedFile);

impl Hash for Specifiers<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(self.0.imports.len());
        for import in &self.0.imports {
            import.specifier.as_str().hash(state);
        }
    }
}

/// What the imports of each of the session's files resolved to, as the cache
/// keeps it.
#[derive(Debug, Clone, PartialEq)]
struct Resolutions {
    /// The [hash](imports_hash) of the imports of each file, or `None` for a
    /// file of a language that doesn't keep what they resolve to.
    imports: Vec<Option<u64>>,

    /// What the imports of each file resolved to, in the order of the
    /// session's files. A file of a language that doesn't keep them has none
    /// in the cache.
    resolved: Vec<Vec<ResolvedImport>>,
}

impl Cached for Resolutions {
    const FILE: &'static str = "resolve.bin";

    fn encode(&self, encoder: &mut Encoder) {
        encoder.length(self.imports.len());
        for (imports, resolved) in self.imports.iter().zip(&self.resolved) {
            encoder.option(imports.as_ref(), |encoder, imports| {
                encoder.u64(*imports);
                encoder.length(resolved.len());
                for resolved in resolved {
                    resolved.encode(encoder);
                }
            });
        }
    }

    fn decode(decoder: &mut Decoder<'_>) -> Option<Self> {
        let len = decoder.length()?;
        let (mut imports, mut resolved) = (Vec::with_capacity(len), Vec::with_capacity(len));
        for _ in 0..len {
            let file = decoder
                .option(|decoder| Some((decoder.u64()?, decoder.list(ResolvedImport::decode)?)))?;
            let (hash, file) = file.unzip();
            imports.push(hash);
            resolved.push(file.unwrap_or_default());
        }
        Some(Self { imports, resolved })
    }
}

impl ResolvedImport {
    /// Write what an import resolved to, as [`ResolvedImport::decode`] reads
    /// it.
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            Self::File(index) => {
                encoder.u8(0);
                encoder.u32(*index);
            }
            Self::Other(resolved) => {
                encoder.u8(1);
                resolved.encode(encoder);
            }
        }
    }

    /// Read what an import resolved to, as [`ResolvedImport::encode`] wrote
    /// it, or `None` if the bytes don't hold it.
    fn decode(decoder: &mut Decoder<'_>) -> Option<Self> {
        match decoder.u8()? {
            0 => Some(Self::File(decoder.u32()?)),
            1 => Some(Self::Other(Resolved::decode(decoder)?)),
            _ => None,
        }
    }
}
