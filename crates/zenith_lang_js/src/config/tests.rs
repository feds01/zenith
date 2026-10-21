//! Parsing the `[languages.js]` table: its options, its defaults, its other
//! name, its errors, and the selector schemes it enables.

use std::path::PathBuf;

use zenith_config::{Config, ContractKind, Languages, Selector};
use zenith_lang::WorkspacePackage;
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

/// The resolver settings of `js` for a project at `/repo` with the
/// `workspace` packages.
fn settings(js: &JsConfig, workspace: &Workspace) -> ResolverSettings {
    js.resolver_settings(Path::new("/repo"), workspace)
}

#[test]
fn the_defaults_resolve_like_typescript() {
    let settings = settings(&js(""), &Workspace::default());
    let defaults = ResolverSettings::new(PathBuf::from("/repo"));
    assert_eq!(settings.root, Path::new("/repo"));
    assert_eq!(settings.tsconfig, TsconfigMode::Auto);
    assert_eq!(settings.conditions, defaults.conditions);
    assert_eq!(settings.main_fields, defaults.main_fields);
    assert_eq!(settings.extensions, defaults.extensions);
    assert!(settings.aliases.is_empty() && settings.ignore_unresolved.is_empty());
    assert!(settings.workspace_packages.is_empty());
}

#[test]
fn the_js_table_replaces_the_resolver_defaults() {
    let options =
        settings(&js(include_str!("fixtures/resolver-options.toml")), &Workspace::default());
    assert_eq!(options.tsconfig, TsconfigMode::File(PathBuf::from("/repo/web/tsconfig.json")));
    assert_eq!(options.conditions, ["browser", "import"]);
    assert_eq!(options.main_fields, ["main"]);
    assert_eq!(options.extensions, [".ts", ".js"]);
    // Aliases keep the order of the file, and point into the root.
    assert_eq!(
        options.aliases,
        [
            ("types".to_string(), PathBuf::from("/repo/web/src/types")),
            ("~".to_string(), PathBuf::from("/repo/web")),
        ]
    );
    let patterns: Vec<_> =
        options.ignore_unresolved.iter().map(|pattern| pattern.as_str()).collect();
    assert_eq!(patterns, ["virtual:*"]);

    let off = settings(&js(include_str!("fixtures/tsconfig-off.toml")), &Workspace::default());
    assert_eq!(off.tsconfig, TsconfigMode::Off);
}

#[test]
fn the_resolver_knows_the_workspace_packages() {
    let package = WorkspacePackage {
        name: "@acme/ui".to_string(),
        dir: "packages/ui".to_string(),
        path: PathBuf::from("/repo/packages/ui"),
    };
    let workspace = Workspace { packages: vec![package], root_package: None };
    assert_eq!(
        settings(&js(""), &workspace).workspace_packages,
        [("@acme/ui".to_string(), PathBuf::from("/repo/packages/ui"))]
    );
}
