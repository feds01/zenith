//! Benchmarks for what is specific to Haskell projects.

use divan::Bencher;
use zenith_lang_haskell::parse::{Dialect, Parser};

// The allocator of the `zenith` binary. CodSpeed measures the allocations
// itself, in its memory mode.
#[cfg(not(codspeed))]
#[global_allocator]
static ALLOC: divan::AllocProfiler<mimalloc::MiMalloc> =
    divan::AllocProfiler::new(mimalloc::MiMalloc);

#[cfg(codspeed)]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The numbers of imports of the files. The parser stops at the first
/// declaration after the imports, so the declarations after them, of which
/// every file has as many, cost nothing.
const IMPORTS: &[usize] = &[1, 10, 100];

/// How many declarations follow the imports.
const DECLARATIONS: usize = 100;

fn main() {
    divan::main();
}

/// A Haskell module with pragmas, a header that exports a few names,
/// `imports` imports of every form, and the declarations after them.
fn haskell_file(imports: usize) -> String {
    let mut source = String::from(
        "{-# LANGUAGE OverloadedStrings #-}\n{-# LANGUAGE ScopedTypeVariables #-}\n\n\
         -- | The handlers of the API.\nmodule Acme.Api.Handlers\n  ( serve\n  , Handler (..)\n  \
         , (<+>)\n  ) where\n\n",
    );
    for index in 0..imports {
        source.push_str(match index % 4 {
            0 => "import qualified Data.Map.Strict as Map\n",
            1 => "import Data.Text (Text, pack, unpack) -- the text type\n",
            2 => "import \"aeson\" Data.Aeson.Types hiding (Parser)\n",
            _ => "import Acme.Api.Types\n  ( Request (..)\n  , Response (..)\n  )\n",
        });
    }
    for index in 0..DECLARATIONS {
        source.push_str(&format!(
            "\n-- | Handle request {index}.\nhandle{index} :: Request -> IO Response\n\
             handle{index} request = do\n  let body = pack (show request)\n  \
             pure Response {{ status = 200, body = body }}\n"
        ));
    }
    source
}

/// Parses one file, with a parser that is reused between files, as each
/// thread of a run does.
#[divan::bench(args = IMPORTS)]
fn parse(bencher: Bencher, imports: usize) {
    let source = haskell_file(imports);
    let mut parser = Parser::new();

    // CodSpeed has no counters, and warns about each one.
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::BytesCount::of_str(&source));

    bencher.bench_local(|| parser.parse(&source, Dialect::PLAIN));
}
