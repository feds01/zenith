//! Benchmarks for checking contracts against the graphs of synthetic
//! repositories.

use divan::Bencher;
use zenith_benchmark::TempRepo;
use zenith_config::Config;
use zenith_contracts::Contract;
use zenith_pipeline::{Pipeline, Session, StageKind};

// CodSpeed measures the allocations itself, in its memory mode.
#[cfg(not(codspeed))]
#[global_allocator]
static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

/// The repository sizes, in source files.
#[cfg(not(codspeed))]
const SIZES: &[usize] = &[1_000, 10_000, 50_000];

/// The repository sizes under CodSpeed, whose simulation would take too long
/// to write and index 50k files.
#[cfg(codspeed)]
const SIZES: &[usize] = &[1_000, 10_000];

/// The contracts that are benchmarked. Every repository has libraries in
/// `packages/`, services in `services/` and apps in `apps/`.
const CONTRACTS: &str = r#"
# Libraries are lower than apps and services, so this searches every pair
# without finding a chain.
[contracts.libraries-below-apps]
type = "forbidden"
from = ["packages/*"]
to = ["apps/*", "services/*"]

# Apps reach builtins through libraries, so this finds chains.
[contracts.apps-without-builtins]
type = "forbidden"
from = ["apps/*"]
to = ["node:*"]

# Libraries import each other, so this finds illegal imports.
[contracts.libraries-for-apps]
type = "protected"
modules = ["packages/*"]
allowed_importers = ["apps/*", "services/*"]
"#;

fn main() {
    let _remove_shared = TempRepo::remove_shared_on_drop();
    divan::main();
}

/// The graph of the shared repository of `source_files` files, configured
/// with the benchmarked contracts.
fn session(source_files: usize) -> Session {
    let repo = TempRepo::shared(source_files);
    let mut session = Session::new(repo.root.clone(), Config::parse(CONTRACTS).unwrap());
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

    bencher.bench(|| contract.check(&session.graph, &session.workspace));
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
