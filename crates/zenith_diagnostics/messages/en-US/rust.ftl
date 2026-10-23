### The messages about Rust: its crates, which Cargo or their manifests
### declare, the syntax of its files, and how their imports resolve.

## Finding the crates.

rust_cargo_not_installed = cargo is not installed
rust_reading_manifests = `cargo` is not installed, so reading the manifests instead
rust_cargo_failed = `cargo metadata` failed, so reading the manifests instead: {$error}
rust_read = could not read `{$path}`: {$error}

## A syntax error of a Rust file that rust-analyzer's parser doesn't report.

rust_source_too_large = the source is too large to read

## Why an import does not resolve, or is left out.

rust_no_package = the file is in no Cargo package
rust_no_module_tree = the file is in no module tree of `{$name}`
rust_super_above_root = `super` goes above the crate root
rust_no_library = `{$name}` has no library
rust_type_in_scope = a type in scope, not a crate or module
rust_tool = an attribute or lint of the tool `{$name}`
rust_not_a_crate = `{$name}` is not a crate, or a module here
rust_importing_file = the path is in the importing file
