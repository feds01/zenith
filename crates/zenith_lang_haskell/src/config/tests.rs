//! Parsing the `[languages.haskell]` table: its options, its defaults, its
//! errors, and the selector schemes it enables.

use zenith_config::{Config, ContractKind, Languages, Selector};
use zenith_utils::glob::{Glob, NamePattern};

use super::*;

/// The languages of the tests: only Haskell.
const LANGUAGES: Languages = Languages::new(&[&HASKELL]);

/// The options of Haskell in a configuration that must be valid.
fn haskell(contents: &str) -> HaskellConfig {
    let config = Config::parse(contents, LANGUAGES).unwrap_or_else(|errors| panic!("{errors:?}"));
    config.languages.options::<HaskellConfig>().expect("haskell is enabled").clone()
}

#[test]
fn haskell_is_enabled_by_writing_its_table() {
    assert!(Config::defaults(LANGUAGES).languages.options::<HaskellConfig>().is_none());
    let haskell = haskell("[languages.haskell]\n");
    assert_eq!((haskell.include, haskell.ignore_unresolved), (None, vec![]));
}

#[test]
fn parses_the_haskell_table() {
    let haskell = haskell(include_str!("fixtures/haskell-table.toml"));
    assert_eq!(haskell.include, Some(vec![Glob::new("services/**").unwrap()]));
    assert_eq!(haskell.ignore_unresolved, [NamePattern::new("Generated.*").unwrap()]);
}

#[test]
fn an_unknown_key_is_an_error() {
    let errors = Config::parse(include_str!("fixtures/unknown-key.toml"), LANGUAGES).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert_eq!(
        errors[0].to_string(),
        "unknown key `zzz`, expected `include` or `ignore_unresolved`"
    );
}

#[test]
fn haskell_enables_the_hackage_scheme() {
    let config = Config::parse(include_str!("fixtures/schemes.toml"), LANGUAGES).unwrap();
    let ContractKind::Forbidden(forbidden) = &config.contracts[0].kind else {
        panic!("expected `forbidden`");
    };
    let schemes: Vec<_> = forbidden.to.iter().map(Selector::scheme).collect();
    assert_eq!(schemes, [Some("hackage"), Some("pkg")]);
}
