//! Benchmarks for finding a project's files and workspace packages, on
//! synthetic repositories written to disk.

use divan::Bencher;
use zenith_benchmark::{TempRepo, default_walk_settings};

// CodSpeed measures the allocations itself, in its memory mode.
#[cfg(not(codspeed))]
#[global_allocator]
static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

/// The repository sizes, in source files.
#[cfg(not(codspeed))]
const SIZES: &[usize] = &[1_000, 10_000, 50_000];

/// The repository sizes under CodSpeed, whose simulation would take too long
/// to write and walk 50k files.
#[cfg(codspeed)]
const SIZES: &[usize] = &[1_000, 10_000];

fn main() {
    let _remove_shared = TempRepo::remove_shared_on_drop();
    divan::main();
}

/// Walks the repository with the default settings, as every command does
/// first.
#[divan::bench(args = SIZES, sample_count = 20)]
fn walk(bencher: Bencher, source_files: usize) {
    let repo = TempRepo::shared(source_files);
    let settings = default_walk_settings();

    // CodSpeed has no counters, and warns about each one.
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::ItemsCount::new(source_files));

    bencher.bench(|| zenith_workspace::walk(&repo.root, &settings));
}

/// Discovers the workspace packages from the `package.json` files that the
/// walk found.
#[divan::bench(args = SIZES)]
fn discover_packages(bencher: Bencher, source_files: usize) {
    let repo = TempRepo::shared(source_files);
    let found = zenith_workspace::walk(&repo.root, &default_walk_settings());

    // CodSpeed has no counters, and warns about each one.
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::ItemsCount::new(found.package_jsons.len()));

    bencher.bench(|| zenith_workspace::discover_packages(&repo.root, &found.package_jsons));
}
