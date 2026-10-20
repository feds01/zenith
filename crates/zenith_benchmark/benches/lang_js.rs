//! Benchmarks for what is specific to JavaScript and TypeScript projects, on
//! synthetic repositories written to disk.

use divan::Bencher;
use zenith_benchmark::{TempRepo, default_walk_settings};

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
/// to write and walk 50k files.
#[cfg(codspeed)]
const SIZES: &[usize] = &[1_000, 10_000];

fn main() {
    let _remove_shared = TempRepo::remove_shared_on_drop();
    divan::main();
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

    bencher.bench(|| zenith_lang_js::discover_packages(&repo.root, &found.package_jsons));
}
