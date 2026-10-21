//! Benchmarks for `zenith check` short of showing its results: the pipeline
//! up to the graph, then compiling and checking every benchmarked contract.

use divan::Bencher;
use zenith_benchmark::TempRepo;
use zenith_config::Config;
use zenith_contracts::{Contract, Exemptions};
use zenith_pipeline::{LANGUAGES, Pipeline, Session, StageKind};

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
/// to write and index 50k files.
#[cfg(codspeed)]
const SIZES: &[usize] = &[1_000, 10_000];

/// The contracts that are checked, one or two of each type, as the
/// `contracts` benchmarks check them one by one.
const CONTRACTS: &str = include_str!("contracts.toml");

fn main() {
    let _remove_shared = TempRepo::remove_shared_on_drop();
    divan::main();
}

/// A session for the shared repository of `source_files` files, configured
/// with the benchmarked contracts.
fn session(source_files: usize) -> Session {
    let repo = TempRepo::shared(source_files);
    Session::new(repo.root.clone(), Config::parse(CONTRACTS, LANGUAGES).unwrap())
}

/// Compile every contract of `session` against its graph, and check them in
/// parallel, without the imports that its suppressions leave out.
fn check_contracts(session: &Session) {
    let contracts =
        Contract::compile_all(&session.config.contracts, &session.graph, &session.workspace)
            .unwrap();
    let exemptions = Exemptions { suppressions: &session.suppressions, baseline: None };
    let reports =
        Contract::check_all(&contracts, session.graphs(), &session.workspace, &exemptions);
    divan::black_box(reports);
}

/// Runs the pipeline up to the graph, then compiles and checks every
/// contract, as `zenith check` does.
#[divan::bench(args = SIZES, sample_count = 10, sample_size = 1)]
fn check(bencher: Bencher, source_files: usize) {
    // CodSpeed has no counters, and warns about each one.
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::ItemsCount::new(source_files));

    bencher.with_inputs(|| session(source_files)).bench_local_refs(|session| {
        Pipeline::standard().run_until(session, StageKind::Graph).unwrap();
        check_contracts(session);
    });
}

/// Compiles and checks every contract on a graph built beforehand, which is
/// the part of `check` after the pipeline.
#[divan::bench(args = SIZES, sample_count = 10)]
fn contracts(bencher: Bencher, source_files: usize) {
    let mut session = session(source_files);
    Pipeline::standard().run_until(&mut session, StageKind::Graph).unwrap();

    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::ItemsCount::new(source_files));

    bencher.bench(|| check_contracts(&session));
}
