//! Benchmarks for the pipeline that turns a project into its import graph,
//! as a whole and stage by stage, on synthetic repositories written to disk,
//! with and without the parse cache.

use std::{
    collections::BTreeMap,
    fs,
    sync::{Mutex, PoisonError},
    thread,
    time::Duration,
};

use divan::Bencher;
use tempfile::TempDir;
use zenith_benchmark::TempRepo;
use zenith_config::Config;
use zenith_pipeline::{
    BuildGraph, CacheSettings, Parse, Pipeline, Resolve, Session, Stage, StageKind,
};

// The allocator of the `zenith` binary. CodSpeed measures the allocations
// itself, in its memory mode.
#[cfg(not(codspeed))]
#[global_allocator]
static ALLOC: divan::AllocProfiler<mimalloc::MiMalloc> =
    divan::AllocProfiler::new(mimalloc::MiMalloc);

#[cfg(codspeed)]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The repository sizes, in source files.
#[cfg(not(codspeed))]
const SIZES: &[usize] = &[1_000, 10_000, 50_000];

/// The repository sizes under CodSpeed, whose simulation would take too long
/// to write and parse 50k files.
#[cfg(codspeed)]
const SIZES: &[usize] = &[1_000, 10_000];

fn main() {
    let _remove_shared = TempRepo::remove_shared_on_drop();
    let _remove_caches = RemoveWarmCaches;
    divan::main();
}

/// A session for the shared repository of `source_files` files, with the
/// standard stages up to `last` run, if any.
fn session(source_files: usize, last: Option<StageKind>) -> Session {
    let repo = TempRepo::shared(source_files);
    let mut session = Session::new(repo.root.clone(), Config::default());
    if let Some(last) = last {
        Pipeline::standard().run_until(&mut session, last).unwrap();
    }
    session
}

/// The parse caches that [`warm_cache`] has written, by repository size.
static WARM_CACHES: Mutex<BTreeMap<usize, TempDir>> = Mutex::new(BTreeMap::new());

/// A guard that removes the directories of the warm caches when it's
/// dropped, even if a benchmark panics.
struct RemoveWarmCaches;

impl Drop for RemoveWarmCaches {
    fn drop(&mut self) {
        WARM_CACHES.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }
}

/// The settings of a parse cache in `dir`.
fn cache_settings(dir: &TempDir) -> CacheSettings {
    CacheSettings { dir: dir.path().to_path_buf(), version: "benchmark".to_string() }
}

/// The settings of a parse cache of the shared repository of
/// `source_files` files, written once, by a run long enough after the
/// repository was written that every file is trusted to be unchanged, as
/// they are after an edit-and-check loop has run a few times.
fn warm_cache(source_files: usize) -> CacheSettings {
    let mut caches = WARM_CACHES.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = caches.entry(source_files).or_insert_with(|| {
        let mut session = session(source_files, Some(StageKind::Discover));
        // Files modified within two seconds of a run that caches them are read again
        // on the next run, so wait out that window.
        thread::sleep(Duration::from_millis(2_100));
        let dir = TempDir::new().unwrap();
        session.cache = Some(cache_settings(&dir));
        Parse.run(&mut session).unwrap();
        dir
    });
    cache_settings(dir)
}

/// Benchmark `stage` alone, on sessions that the stages before it, up to
/// `before`, have run on.
fn bench_stage(bencher: Bencher, source_files: usize, before: StageKind, stage: &dyn Stage) {
    // CodSpeed has no counters, and warns about each one.
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::ItemsCount::new(source_files));

    bencher
        .with_inputs(|| session(source_files, Some(before)))
        .bench_local_refs(|session| stage.run(session).unwrap());
}

/// Runs every stage up to the graph with the default configuration, as the
/// commands that need the graph do first.
#[divan::bench(args = SIZES, sample_count = 10, sample_size = 1)]
fn pipeline(bencher: Bencher, source_files: usize) {
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::ItemsCount::new(source_files));

    bencher.with_inputs(|| session(source_files, None)).bench_local_refs(|session| {
        Pipeline::standard().run_until(session, StageKind::Graph).unwrap();
    });
}

/// Parses every source file.
#[divan::bench(args = SIZES, sample_count = 10, sample_size = 1)]
fn parse(bencher: Bencher, source_files: usize) {
    bench_stage(bencher, source_files, StageKind::Discover, &Parse);
}

/// Resolves every import.
#[divan::bench(args = SIZES, sample_count = 10, sample_size = 1)]
fn resolve(bencher: Bencher, source_files: usize) {
    bench_stage(bencher, source_files, StageKind::Parse, &Resolve);
}

/// Builds the graph from the parsed files and their resolved imports.
#[divan::bench(args = SIZES, sample_count = 10, sample_size = 1)]
fn graph(bencher: Bencher, source_files: usize) {
    bench_stage(bencher, source_files, StageKind::Resolve, &BuildGraph);
}

/// Parses every source file without a parse cache, and writes one, as the
/// first run on a project does.
#[divan::bench(args = SIZES, sample_count = 10, sample_size = 1)]
fn parse_cold_cache(bencher: Bencher, source_files: usize) {
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::ItemsCount::new(source_files));

    let dir = TempDir::new().unwrap();
    let cache = cache_settings(&dir);
    bencher
        .with_inputs(|| {
            // Only the cache file is removed: creating its directory is a one-off.
            let _ = fs::remove_file(cache.file());
            let mut session = session(source_files, Some(StageKind::Discover));
            session.cache = Some(cache.clone());
            session
        })
        .bench_local_refs(|session| Parse.run(session).unwrap());
}

/// Takes every source file from a parse cache that has all of them, as a run
/// on a project that hasn't changed does.
#[divan::bench(args = SIZES, sample_count = 10, sample_size = 1)]
fn parse_warm_cache(bencher: Bencher, source_files: usize) {
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::ItemsCount::new(source_files));

    let cache = warm_cache(source_files);
    bencher
        .with_inputs(|| {
            let mut session = session(source_files, Some(StageKind::Discover));
            session.cache = Some(cache.clone());
            session
        })
        .bench_local_refs(|session| {
            Parse.run(session).unwrap();
            assert_eq!(session.stats.cached_files, source_files);
        });
}

/// Runs every stage up to the graph with a parse cache that has every file,
/// as a command on a project that hasn't changed does.
#[divan::bench(args = SIZES, sample_count = 10, sample_size = 1)]
fn pipeline_warm_cache(bencher: Bencher, source_files: usize) {
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::ItemsCount::new(source_files));

    let cache = warm_cache(source_files);
    bencher
        .with_inputs(|| {
            let mut session = session(source_files, None);
            session.cache = Some(cache.clone());
            session
        })
        .bench_local_refs(|session| {
            Pipeline::standard().run_until(session, StageKind::Graph).unwrap();
        });
}
