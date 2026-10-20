//! Benchmarks for what is specific to Go projects.

use divan::Bencher;
use zenith_lang_go::parse::Parser;

// The allocator of the `zenith` binary. CodSpeed measures the allocations
// itself, in its memory mode.
#[cfg(not(codspeed))]
#[global_allocator]
static ALLOC: divan::AllocProfiler<mimalloc::MiMalloc> =
    divan::AllocProfiler::new(mimalloc::MiMalloc);

#[cfg(codspeed)]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The sizes of the files, in functions after the imports. The parser reads
/// the whole file, so its time grows with them, though only the imports are
/// kept.
const FUNCTIONS: &[usize] = &[0, 10, 100];

fn main() {
    divan::main();
}

/// A Go file with a package clause, a group of imports, and `functions`
/// functions after them.
fn go_file(functions: usize) -> String {
    let mut source = String::from(
        "// Package server serves things.\npackage server\n\nimport (\n\t\"context\"\n\t\"fmt\"\n\
         \tlog \"github.com/acme/logging\"\n\t_ \"embed\"\n)\n",
    );
    for index in 0..functions {
        source.push_str(&format!(
            "\n// Handle{index} handles a request.\n\
             func Handle{index}(ctx context.Context, name string) (int, error) {{\n\
             \tif name == \"\" {{\n\t\treturn 0, fmt.Errorf(\"no name in %d\", {index})\n\t}}\n\
             \tfor i := 0; i < len(name); i++ {{\n\t\tlog.Info(ctx, name[i:])\n\t}}\n\
             \treturn len(name), nil\n}}\n"
        ));
    }
    source
}

/// Parses one file, with a parser that is reused between files, as each
/// thread of a run does.
#[divan::bench(args = FUNCTIONS)]
fn parse(bencher: Bencher, functions: usize) {
    let source = go_file(functions);
    let mut parser = Parser::new();

    // CodSpeed has no counters, and warns about each one.
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::BytesCount::of_str(&source));

    bencher.bench_local(|| parser.parse(&source));
}
