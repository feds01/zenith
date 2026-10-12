//! Benchmarks for the pipeline that turns a project into its import graph,
//! as a whole and stage by stage, on synthetic repositories written to disk.

use divan::Bencher;
use zenith_benchmark::TempRepo;
use zenith_config::Config;
use zenith_pipeline::{BuildGraph, Parse, Pipeline, Resolve, Session, Stage, StageKind};

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
