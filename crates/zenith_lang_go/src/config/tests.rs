//! Parsing the `[languages.go]` table: its options, its defaults, its errors,
//! and the selector schemes it enables.

use zenith_config::{Config, ContractKind, Languages, Selector};
use zenith_utils::glob::{Glob, NamePattern};

use super::*;

/// The languages of the tests: only Go.
const LANGUAGES: Languages = Languages::new(&[&GO]);

/// The options of Go in a configuration that must be valid.
fn go(contents: &str) -> GoConfig {
    let config = Config::parse(contents, LANGUAGES).unwrap_or_else(|errors| panic!("{errors:?}"));
    config.languages.options::<GoConfig>().expect("go is enabled").clone()
}

/// The only error of an invalid configuration.
fn only_error(contents: &str) -> String {
    let errors = Config::parse(contents, LANGUAGES).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:#?}");
    errors[0].to_string()
}

#[test]
fn go_is_enabled_by_its_table_alone() {
    assert!(Config::defaults(LANGUAGES).languages.options::<GoConfig>().is_none());
    let go = go("[languages.go]\n");
    assert_eq!((go.include, go.ignore_unresolved), (None, vec![]));
}

#[test]
fn parses_the_go_table() {
    let go = go(include_str!("fixtures/go-table.toml"));
    assert_eq!(
        go.include,
        Some(vec![Glob::new("cmd/**").unwrap(), Glob::new("internal/**/*.go").unwrap()])
    );
    assert_eq!(go.ignore_unresolved, [NamePattern::new("example.com/generated/*").unwrap()]);
}

#[test]
fn invalid_tables_are_errors() {
    for (contents, expected) in [
        (
            include_str!("fixtures/unknown-key.toml"),
            "unknown key `zzz`, expected `include` or `ignore_unresolved`",
        ),
        (
            include_str!("fixtures/invalid-pattern.toml"),
            "invalid pattern `internal/{`: unclosed `{`",
        ),
    ] {
        assert_eq!(only_error(contents), expected, "{contents}");
    }
}

#[test]
fn go_enables_the_go_and_gomod_schemes() {
    let config = Config::parse(include_str!("fixtures/schemes.toml"), LANGUAGES).unwrap();
    let ContractKind::Forbidden(forbidden) = &config.contracts[0].kind else {
        panic!("expected `forbidden`");
    };
    let schemes: Vec<_> = forbidden.to.iter().map(Selector::scheme).collect();
    assert_eq!(schemes, [Some("go"), Some("gomod"), Some("pkg")]);
}
