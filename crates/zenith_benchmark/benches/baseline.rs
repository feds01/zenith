//! Benchmarks for recording a baseline: finding the entries that keep every
//! benchmarked contract, as `zenith baseline` does after the pipeline.

use divan::Bencher;
use zenith_benchmark::TempRepo;
use zenith_config::Config;
use zenith_contracts::{Baseline, Contract, Exemptions};
use zenith_pipeline::{Pipeline, Session, StageKind};

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

/// The contracts whose baseline is recorded, one or two of each type, as the
/// `contracts` benchmarks check them one by one.
const CONTRACTS: &str = include_str!("contracts.toml");

fn main() {
    let _remove_shared = TempRepo::remove_shared_on_drop();
    divan::main();
}

/// Records the entries of every contract in an empty baseline: each contract
/// is checked again and again, its violations' imports left out each time,
/// until it finds no more.
#[divan::bench(args = SIZES, sample_count = 10)]
fn record(bencher: Bencher, source_files: usize) {
    let repo = TempRepo::shared(source_files);
    let mut session = Session::new(repo.root.clone(), Config::parse(CONTRACTS).unwrap());
    Pipeline::standard().run_until(&mut session, StageKind::Graph).unwrap();
    let contracts =
        Contract::compile_all(&session.config.contracts, &session.graph, &session.workspace)
            .unwrap();
    let exemptions = Exemptions { suppressions: &session.suppressions, baseline: None };

    // CodSpeed has no counters, and warns about each one.
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::ItemsCount::new(source_files));

    bencher.bench(|| {
        let mut baseline = Baseline::default();
        baseline.record(&contracts, session.graphs(), &session.workspace, &exemptions);
        baseline
    });
}
