//! Parsing the `[languages.python]` table: its options, its defaults, its
//! errors, and the selector schemes it enables.

use zenith_config::{Config, ContractKind, Languages, Selector};
use zenith_utils::glob::{Glob, NamePattern};

use super::*;

/// The languages of the tests: only Python.
const LANGUAGES: Languages = Languages::new(&[&PYTHON]);

/// The options of Python in a configuration that must be valid.
fn python(contents: &str) -> PythonConfig {
    let config = Config::parse(contents, LANGUAGES).unwrap_or_else(|errors| panic!("{errors:?}"));
    config.languages.options::<PythonConfig>().expect("python is enabled").clone()
}

#[test]
fn python_is_enabled_by_writing_its_table() {
    let python = python("[languages.python]\n");
    assert_eq!((python.include, python.roots, python.ignore_unresolved), (None, None, vec![]));
}

#[test]
fn parses_the_python_table() {
    let python = python(include_str!("fixtures/python-table.toml"));
    assert_eq!(python.include, Some(vec![Glob::new("services/**").unwrap()]));
    assert_eq!(python.roots, Some(vec!["services/api/src".to_string(), ".".to_string()]));
    assert_eq!(python.ignore_unresolved, [NamePattern::new("generated.*").unwrap()]);
}

#[test]
fn an_unknown_key_is_an_error() {
    let errors = Config::parse(include_str!("fixtures/unknown-key.toml"), LANGUAGES).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert_eq!(
        errors[0].to_string(),
        "unknown key `zzz`, expected one of `include`, `roots`, `ignore_unresolved`"
    );
}

#[test]
fn python_enables_the_pypi_and_python_schemes() {
    let config = Config::parse(include_str!("fixtures/schemes.toml"), LANGUAGES).unwrap();
    let ContractKind::Forbidden(forbidden) = &config.contracts[0].kind else {
        panic!("expected `forbidden`");
    };
    let schemes: Vec<_> = forbidden.to.iter().map(Selector::scheme).collect();
    assert_eq!(schemes, [Some("pypi"), Some("python"), Some("pkg")]);
}
