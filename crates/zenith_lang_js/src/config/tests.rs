//! Parsing the `[languages.js]` table: its options, its defaults, its other
//! name, its errors, and the selector schemes it enables.

use zenith_config::{Config, ContractKind, Languages, Selector};
use zenith_utils::glob::{Glob, NamePattern};

use super::*;

/// The languages of the tests: only JavaScript and TypeScript.
const LANGUAGES: Languages = Languages::new(&[&JS]);

/// The options of JavaScript and TypeScript in a configuration that must be
/// valid.
fn js(contents: &str) -> JsConfig {
    let config = Config::parse(contents, LANGUAGES).unwrap_or_else(|errors| panic!("{errors:?}"));
    config.languages.options::<JsConfig>().expect("js is enabled").clone()
}

/// The only error of an invalid configuration.
fn only_error(contents: &str) -> String {
    let errors = Config::parse(contents, LANGUAGES).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:#?}");
    errors[0].to_string()
}

#[test]
fn js_is_enabled_by_default_with_its_defaults() {
    for js in [js(""), js("[languages.js]\n")] {
        assert_eq!(js.include, None);
        assert_eq!(js.tsconfig, Tsconfig::Auto);
        assert_eq!(js.conditions, None);
        assert_eq!(js.main_fields, None);
        assert_eq!(js.extensions, None);
        assert!(js.alias.is_empty());
        assert!(js.ignore_unresolved.is_empty());
    }
}

#[test]
fn parses_the_js_table() {
    let js = js(include_str!("fixtures/js-table.toml"));
    assert_eq!(js.include, Some(vec![Glob::new("**/*.{ts,tsx}").unwrap()]));
    assert_eq!(js.tsconfig, Tsconfig::File("config/tsconfig.base.json".into()));
    assert_eq!(js.conditions.as_deref(), Some(&["source".to_string(), "import".to_string()][..]));
    assert_eq!(js.main_fields.as_deref(), Some(&["main".to_string()][..]));
    assert_eq!(js.extensions.as_deref(), Some(&[".ts".to_string(), ".js".to_string()][..]));
    // Aliases keep the order of the file.
    let alias: Vec<_> = js.alias.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(alias, [("types", "web/src/types"), ("~", "web/src")]);
    assert_eq!(
        js.ignore_unresolved,
        [NamePattern::new("/*").unwrap(), NamePattern::new("virtual:*").unwrap()]
    );
}

#[test]
fn tsconfig_can_be_automatic_or_off() {
    assert_eq!(js(include_str!("fixtures/tsconfig-auto.toml")).tsconfig, Tsconfig::Auto);
    assert_eq!(js(include_str!("fixtures/tsconfig-off.toml")).tsconfig, Tsconfig::Off);
}

#[test]
fn the_js_table_is_also_called_typescript() {
    let js = js(include_str!("fixtures/typescript-table.toml"));
    assert_eq!(js.include, Some(vec![Glob::new("web/**").unwrap()]));
    assert_eq!(js.tsconfig, Tsconfig::Off);
    assert_eq!(js.ignore_unresolved, [NamePattern::new("virtual:*").unwrap()]);
}

#[test]
fn invalid_tables_are_errors() {
    for (contents, expected) in [
        (
            include_str!("fixtures/unknown-key.toml"),
            "unknown key `zzz`, expected one of `include`, `tsconfig`, `conditions`, \
             `main_fields`, `extensions`, `alias`, `ignore_unresolved`",
        ),
        (include_str!("fixtures/invalid-alias.toml"), "invalid type: array, expected a table"),
        (
            include_str!("fixtures/invalid-pattern.toml"),
            "invalid pattern `virtual:{`: unclosed `{`",
        ),
    ] {
        assert_eq!(only_error(contents), expected, "{contents}");
    }
}

#[test]
fn js_enables_the_npm_node_and_bun_schemes() {
    let config = Config::parse(include_str!("fixtures/schemes.toml"), LANGUAGES).unwrap();
    let ContractKind::Forbidden(forbidden) = &config.contracts[0].kind else {
        panic!("expected `forbidden`");
    };
    let schemes: Vec<_> = forbidden.to.iter().map(Selector::scheme).collect();
    assert_eq!(schemes, [Some("npm"), Some("node"), Some("bun"), Some("pkg")]);
}
