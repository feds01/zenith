//! Benchmarks for the glob dialect and name patterns, which selectors and the
//! walk's include and exclude filters are written in.

use std::{fmt, sync::LazyLock};

use divan::{Bencher, black_box};
use zenith_benchmark::SyntheticRepo;
use zenith_utils::glob::{Glob, NamePattern};

// The allocator of the `zenith` binary. CodSpeed measures the allocations
// itself, in its memory mode.
#[cfg(not(codspeed))]
#[global_allocator]
static ALLOC: divan::AllocProfiler<mimalloc::MiMalloc> =
    divan::AllocProfiler::new(mimalloc::MiMalloc);

#[cfg(codspeed)]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The repository whose paths and package names the patterns are matched
/// against. Matching doesn't read from disk, so it stays in memory.
static REPO: LazyLock<SyntheticRepo> = LazyLock::new(|| SyntheticRepo::new(10_000));

fn main() {
    divan::main();
}

/// A pattern that a benchmark runs on, reported under its name.
struct Case {
    /// The name that the benchmark reports the pattern under.
    name: &'static str,

    /// The pattern.
    pattern: &'static str,
}

impl fmt::Display for Case {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name)
    }
}

/// Globs like those written in selectors and file filters, from a literal path
/// to several `**` segments.
const GLOBS: &[Case] = &[
    Case { name: "literal", pattern: "packages/core/src/index.ts" },
    Case { name: "star", pattern: "packages/*/src/index.ts" },
    Case { name: "trailing_recursive", pattern: "packages/*/src/**" },
    Case { name: "extensions", pattern: "**/*.{ts,tsx}" },
    Case { name: "tests", pattern: "**/__tests__/*.test.ts" },
    Case { name: "two_recursive", pattern: "**/features/**/hooks/use-*.ts" },
    Case { name: "route_groups", pattern: "apps/*/src/app/(*)/**/page.tsx" },
];

/// Name patterns like those that select packages and npm imports.
const NAME_PATTERNS: &[Case] = &[
    Case { name: "literal", pattern: "react" },
    Case { name: "scope", pattern: "@acme/*" },
    Case { name: "alternation", pattern: "{react,react-dom,next}" },
    Case { name: "infix", pattern: "@acme/*-client" },
    Case { name: "everything", pattern: "*" },
];

/// Compiles the glob, as loading a config does for every selector.
#[divan::bench(args = GLOBS)]
fn glob_new(case: &Case) -> Glob {
    Glob::new(black_box(case.pattern)).unwrap()
}

/// Matches the glob against the path of every source file of a 10k-file
/// repository, as resolving a selector does.
#[divan::bench(args = GLOBS)]
fn glob_is_match(bencher: Bencher, case: &Case) {
    let glob = Glob::new(case.pattern).unwrap();
    let paths: Vec<&str> = REPO.source_files().map(|file| file.path.as_str()).collect();

    // CodSpeed has no counters, and warns about each one.
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::ItemsCount::new(paths.len()));

    bencher.bench(|| paths.iter().filter(|path| glob.is_match(black_box(path))).count());
}

/// Matches the name pattern against every workspace and npm package name of
/// a 10k-file repository.
#[divan::bench(args = NAME_PATTERNS)]
fn name_pattern_is_match(bencher: Bencher, case: &Case) {
    let pattern = NamePattern::new(case.pattern).unwrap();
    let workspace = REPO.packages().iter().map(|package| package.name.as_str());
    let names: Vec<&str> = workspace.chain(REPO.externals().iter().map(String::as_str)).collect();

    // CodSpeed has no counters, and warns about each one.
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::ItemsCount::new(names.len()));

    bencher.bench(|| names.iter().filter(|name| pattern.is_match(black_box(name))).count());
}
