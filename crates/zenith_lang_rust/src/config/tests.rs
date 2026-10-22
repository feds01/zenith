//! Parsing the `[languages.rust]` table: its options, its defaults, its
//! errors, and the selector schemes it enables.

use zenith_config::{Config, ContractKind, Languages, Selector};
use zenith_utils::glob::Glob;

use super::*;

/// The languages of the tests: only Rust.
const LANGUAGES: Languages = Languages::new(&[&RUST]);

/// The options of Rust in a configuration that must be valid.
fn rust(contents: &str) -> RustConfig {
    let config = Config::parse(contents, LANGUAGES).unwrap_or_else(|errors| panic!("{errors:?}"));
    config.languages.options::<RustConfig>().expect("rust is enabled").clone()
}

#[test]
fn rust_is_enabled_by_writing_its_table() {
    assert_eq!(rust("[languages.rust]\n").include, None);
}

#[test]
fn parses_the_rust_table() {
    let rust = rust(include_str!("fixtures/rust-table.toml"));
    assert_eq!(rust.include, Some(vec![Glob::new("crates/**").unwrap()]));
}

#[test]
fn an_unknown_key_is_an_error() {
    let errors = Config::parse(include_str!("fixtures/unknown-key.toml"), LANGUAGES).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert_eq!(errors[0].to_string(), "unknown key `zzz`, expected `include`");
}

#[test]
fn rust_enables_the_crate_and_rust_schemes() {
    let config = Config::parse(include_str!("fixtures/schemes.toml"), LANGUAGES).unwrap();
    let ContractKind::Forbidden(forbidden) = &config.contracts[0].kind else {
        panic!("expected `forbidden`");
    };
    let schemes: Vec<_> = forbidden.to.iter().map(Selector::scheme).collect();
    assert_eq!(schemes, [Some("crate"), Some("rust"), Some("pkg")]);
}
