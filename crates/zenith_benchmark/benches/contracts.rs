//! Benchmarks for checking contracts against the graphs of synthetic
//! repositories.

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

/// The contracts that are benchmarked. Every repository has libraries in
/// `packages/`, services in `services/` and apps in `apps/`.
const CONTRACTS: &str = include_str!("contracts.toml");

fn main() {
    let _remove_shared = TempRepo::remove_shared_on_drop();
    divan::main();
}

/// The graph of the shared repository of `source_files` files, configured
/// with the benchmarked contracts.
fn session(source_files: usize) -> Session {
    let repo = TempRepo::shared(source_files);
    let mut session = Session::new(repo.root.clone(), Config::parse(CONTRACTS, LANGUAGES).unwrap());
    Pipeline::standard().run_until(&mut session, StageKind::Graph).unwrap();
    session
}

/// Benchmark checking the contract `id` on the repository of `source_files`
/// files, compiled beforehand.
fn bench_contract(bencher: Bencher, source_files: usize, id: &str) {
    let session = session(source_files);
    let config = session.config.contract(id).unwrap();
    let contract = Contract::compile(config, &session.graph, &session.workspace).unwrap();

    // CodSpeed has no counters, and warns about each one.
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::ItemsCount::new(source_files));

    let exemptions = Exemptions { suppressions: &session.suppressions, baseline: None };
    bencher.bench(|| contract.check(&session.graph, &session.workspace, &exemptions));
}

/// A `forbidden` contract that every chain search rules out.
#[divan::bench(args = SIZES, sample_count = 10)]
fn forbidden_without_chains(bencher: Bencher, source_files: usize) {
    bench_contract(bencher, source_files, "libraries-below-apps");
}

/// A `forbidden` contract that finds chains.
#[divan::bench(args = SIZES, sample_count = 10)]
fn forbidden_with_chains(bencher: Bencher, source_files: usize) {
    bench_contract(bencher, source_files, "apps-without-builtins");
}

/// A `protected` contract that finds illegal imports.
#[divan::bench(args = SIZES, sample_count = 10)]
fn protected(bencher: Bencher, source_files: usize) {
    bench_contract(bencher, source_files, "libraries-for-apps");
}

/// An `independence` contract that finds dependencies.
#[divan::bench(args = SIZES, sample_count = 10)]
fn independence(bencher: Bencher, source_files: usize) {
    bench_contract(bencher, source_files, "independent-libraries");
}

/// A `layers` contract of two levels, the upper one independent.
#[divan::bench(args = SIZES, sample_count = 10)]
fn layers(bencher: Bencher, source_files: usize) {
    bench_contract(bencher, source_files, "tiers");
}

/// An `acyclic` contract on the libraries, and two generations of their
/// children.
#[divan::bench(args = SIZES, sample_count = 10)]
fn acyclic(bencher: Bencher, source_files: usize) {
    bench_contract(bencher, source_files, "acyclic-libraries");
}

/// An `encapsulation` contract that finds deep imports.
#[divan::bench(args = SIZES, sample_count = 10)]
fn encapsulation(bencher: Bencher, source_files: usize) {
    bench_contract(bencher, source_files, "library-barrels");
}
