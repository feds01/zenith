//! Benchmarks for what is specific to Terraform projects.

use divan::Bencher;
use zenith_lang_terraform::parse::{Dialect, Parser};

// The allocator of the `zenith` binary. CodSpeed measures the allocations
// itself, in its memory mode.
#[cfg(not(codspeed))]
#[global_allocator]
static ALLOC: divan::AllocProfiler<mimalloc::MiMalloc> =
    divan::AllocProfiler::new(mimalloc::MiMalloc);

#[cfg(codspeed)]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The sizes of the files, in resources after the modules. The parser reads
/// the whole file, so its time grows with them, though only the module
/// sources are kept.
const RESOURCES: &[usize] = &[0, 10, 100];

fn main() {
    divan::main();
}

/// A Terraform file with a `terraform` block, a few modules, and `resources`
/// resources after them.
fn terraform_file(resources: usize) -> String {
    let mut source = String::from(
        "# The production stack.\nterraform {\n  required_providers {\n    aws = {\n      \
         source  = \"hashicorp/aws\"\n      version = \"~> 5.0\"\n    }\n  }\n}\n\n\
         module \"vpc\" {\n  source = \"../../modules/vpc\" # the network\n  cidr   = \
         \"10.0.0.0/16\"\n}\n\nmodule \"consul\" {\n  source  = \"hashicorp/consul/aws\"\n  \
         version = \"0.1.0\"\n}\n",
    );
    for index in 0..resources {
        source.push_str(&format!(
            "\n# The bucket of service {index}.\n\
             resource \"aws_s3_bucket\" \"service_{index}\" {{\n\
             \x20 bucket = \"service-{index}-${{var.environment}}\"\n\
             \x20 tags   = merge(local.tags, {{ Service = \"service-{index}\" }})\n\
             \x20 lifecycle {{\n    prevent_destroy = true\n  }}\n}}\n"
        ));
    }
    source
}

/// Parses one file, with a parser that is reused between files, as each
/// thread of a run does.
#[divan::bench(args = RESOURCES)]
fn parse(bencher: Bencher, resources: usize) {
    let source = terraform_file(resources);
    let mut parser = Parser::new();

    // CodSpeed has no counters, and warns about each one.
    #[cfg(not(codspeed))]
    let bencher = bencher.counter(divan::counter::BytesCount::of_str(&source));

    bencher.bench_local(|| parser.parse(&source, Dialect::Terraform));
}
