//! Benchmarks for what is specific to Python projects.

use std::path::Path;

use divan::Bencher;
use zenith_lang_python::parse::Parser;

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

/// A Python module with a docstring, each kind of import, and `functions`
/// functions after them, each with an import of its own, as code that defers
/// an import to break a cycle has.
fn python_file(functions: usize) -> String {
    let mut source = String::from(
        "\"\"\"Serves things.\"\"\"\n\nfrom __future__ import annotations\n\nimport os\n\
         from typing import TYPE_CHECKING\n\nimport requests\n\n\
         from acme.core.db import Session, engine\nfrom . import models\n\
         from ..util import (\n    retry,  # zenith-ignore layers: legacy\n    timed,\n)\n\n\
         if TYPE_CHECKING:\n    from acme.billing import Invoice\n",
    );
    for index in 0..functions {
        source.push_str(&format!(
            "\n\ndef handle_{index}(session: Session, name: str) -> int:\n\
             \x20   \"\"\"Handle request {index}.\"\"\"\n\
             \x20   from acme.core import cache\n\
             \x20   if not name:\n\
             \x20       raise ValueError(f\"no name in {{{index}}}\")\n\
             \x20   for i, char in enumerate(name):\n\
             \x20       cache.put(os.path.join(name, char), i)\n\
             \x20   return len(name)\n"
        ));
    }
    source
}

/// Parses one file, with a parser that is reused between files, as each
/// thread of a run does.
#[divan::bench(args = FUNCTIONS)]
fn parse(bencher: Bencher, functions: usize) {
    let source = python_file(functions);
    let mut parser = Parser::new();

    // CodSpeed has no counters, and warns about each one.
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::BytesCount::of_str(&source));

    bencher.bench_local(|| parser.parse_source(Path::new("server.py"), &source));
}
