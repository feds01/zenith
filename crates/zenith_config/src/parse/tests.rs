//! Parsing and validating `zenith.toml`: every table and contract type, the
//! defaults, and the errors with their locations.

use std::ops::Range;

use zenith_graph::KindSet;
use zenith_utils::{
    glob::{Glob, GlobError, NamePattern},
    location::ByteRange,
};

use crate::{
    config::{Config, StaleLevel},
    contracts::{AlertLevel, CONTRACT_TYPES, ContractKind, Reexports, Severity},
    error::{ConfigError, ConfigErrorKind, InContract, LoadError, SelectorError},
    selector::{Selector, SelectorTarget},
    testing::{LANGUAGES, OTHER, TEST, TestOptions},
};

/// Parse a configuration that must be valid.
fn valid(contents: &str) -> Config {
    match Config::parse(contents, LANGUAGES) {
        Ok(config) => config,
        Err(errors) => panic!("unexpected errors: {}", render(contents, &errors).join("\n")),
    }
}

/// Parse a configuration that must be invalid, and render its errors as
/// `line:column: message [the offending text]`.
fn invalid(contents: &str) -> Vec<String> {
    match Config::parse(contents, LANGUAGES) {
        Ok(config) => panic!("expected errors, got {config:?}"),
        Err(errors) => render(contents, &errors),
    }
}

/// Render errors as `line:column: message [the offending text]`.
fn render(contents: &str, errors: &[ConfigError]) -> Vec<String> {
    errors
        .iter()
        .map(|error| match error.range {
            Some(range) => {
                let (line, column) = line_column(contents, range);
                format!("{line}:{column}: {error} [{}]", &contents[Range::from(range)])
            }
            None => format!("{error}"),
        })
        .collect()
}

/// The 1-based line and column, in characters, of the start of `range`.
fn line_column(contents: &str, range: ByteRange) -> (usize, usize) {
    let before = &contents[..range.start() as usize];
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    (before.matches('\n').count() + 1, before[line_start..].chars().count() + 1)
}

/// The only error of an invalid configuration.
fn only_error(contents: &str) -> String {
    let errors = invalid(contents);
    assert_eq!(errors.len(), 1, "{errors:#?}");
    errors.into_iter().next().unwrap_or_default()
}

/// The strings of a list of selectors.
fn selectors(selectors: &[Selector]) -> Vec<&str> {
    selectors.iter().map(|selector| selector.as_str()).collect()
}

#[test]
fn every_contract_type_is_parsed() {
    for name in CONTRACT_TYPES {
        let options = match name {
            "forbidden" => "from = []\nto = []",
            "layers" => "layers = [\"a\"]",
            "protected" => "modules = []\nallowed_importers = []",
            "limits" => "modules = []\nmax_fan_in = 1",
            _ => "modules = []",
        };
        let contents = format!("[contracts.c]\ntype = \"{name}\"\n{options}\n");
        let contract = &valid(&contents).contracts[0];
        assert_eq!(contract.kind.type_name(), name);
        // The contract is where its id is, in the header of its table.
        assert_eq!(&contents[Range::from(contract.range)], "c");
    }
}

#[test]
fn an_empty_file_has_the_defaults_of_no_file() {
    let parsed =
        ["", "[files]\n[analysis]\n[baseline]\n[cache]\n[contracts]\n", "[languages.test]\n"]
            .map(valid);
    for config in [Config::defaults(LANGUAGES)].iter().chain(&parsed) {
        assert!(config.files.exclude.is_empty());
        assert!(config.files.respect_gitignore);
        let test = config.languages.options::<TestOptions>().expect("test is enabled");
        assert_eq!(test.include, None);
        assert!(test.names.is_empty());
        assert!(config.analysis.exclude_kinds.is_empty());
        assert_eq!(config.baseline.path, "zenith.baseline.toml");
        assert_eq!(config.baseline.stale, StaleLevel::Error);
        assert!(config.cache.enabled);
        assert_eq!(config.cache.dir, ".zenith_cache");
        assert!(config.contracts.is_empty());
    }
}

#[test]
fn without_a_languages_table_the_default_languages_are_enabled() {
    for config in [Config::defaults(LANGUAGES), valid("")] {
        assert_eq!(config.languages.enabled().collect::<Vec<_>>(), [&TEST]);
        assert!(config.languages.is_enabled(&TEST));
        assert!(!config.languages.is_enabled(&OTHER));
        let scanned = config.languages.scanned();
        assert!(scanned.globs.is_empty());
        assert_eq!(scanned.extensions, TEST.source_extensions);
        assert_eq!(config.languages.optional_extensions(), TEST.optional_extensions);
    }
}

#[test]
fn the_enabled_languages_are_in_the_order_of_the_registry() {
    let config = valid(include_str!("fixtures/languages-reversed.toml"));
    assert_eq!(config.languages.enabled().collect::<Vec<_>>(), [&TEST, &OTHER]);
    // The extensions and file names of each are scanned, and the extensions may
    // be left off selectors.
    let scanned = config.languages.scanned();
    assert_eq!((scanned.extensions, scanned.file_names), (vec!["tst", "oth"], vec!["Otherfile"]));
    assert_eq!(config.languages.optional_extensions(), ["tst", "d.tst", "oth"]);

    // A file is of the language whose extension or file name it has.
    let language_of = |name| config.languages.language_of(name);
    assert_eq!(language_of("src/a.tst"), Some(0));
    assert_eq!(language_of("src/a.oth"), Some(1));
    assert_eq!(language_of("src/Otherfile"), Some(1));
    assert_eq!(language_of("Otherfile"), Some(1));
    assert_eq!(language_of("src/Otherfile.txt"), None);
    assert_eq!(language_of("Otherfile/a.txt"), None);
}

#[test]
fn parses_the_global_tables() {
    let config = valid(include_str!("fixtures/global-tables.toml"));

    let globs =
        |patterns: &[&str]| patterns.iter().map(|p| Glob::new(p).unwrap()).collect::<Vec<_>>();
    assert_eq!(config.files.exclude, globs(&["experimental/**", "**/*.gen.ts"]));
    assert!(!config.files.respect_gitignore);

    let test = config.languages.options::<TestOptions>().unwrap();
    assert_eq!(test.include, Some(globs(&["**/*.tst"])));
    assert_eq!(
        test.names,
        [NamePattern::new("/*").unwrap(), NamePattern::new("virtual:*").unwrap()]
    );

    assert_eq!(config.analysis.exclude_kinds, KindSet::TYPE | KindSet::DYNAMIC);
    assert_eq!(config.baseline.path, "config/zenith.baseline.toml");
    assert_eq!(config.baseline.stale, StaleLevel::Warn);
    assert!(!config.cache.enabled);
    assert_eq!(config.cache.dir, ".cache/zenith");

    // A language that sets its `include` is scanned by its globs.
    let scanned = config.languages.scanned();
    assert_eq!(scanned.globs, globs(&["**/*.tst"]));
    assert!(scanned.extensions.is_empty());
}

#[test]
fn a_language_table_goes_by_any_of_its_names() {
    let config = valid(include_str!("fixtures/languages-other-name.toml"));
    let test = config.languages.options::<TestOptions>().expect("`testing` enables test");
    assert_eq!(test.include, Some(vec![Glob::new("web/**").unwrap()]));
    assert_eq!(test.names, [NamePattern::new("virtual:*").unwrap()]);
}

#[test]
fn giving_a_language_under_two_names_is_an_error() {
    // The second table is the one reported, in either order.
    let contents = include_str!("fixtures/languages-both-names.toml");
    assert_eq!(
        invalid(contents),
        ["4:12: `languages.testing` is another name for `languages.test`, which is already given \
          [testing]"]
    );
    let contents = include_str!("fixtures/languages-both-names-reversed.toml");
    assert_eq!(
        invalid(contents),
        ["4:12: `languages.test` is another name for `languages.testing`, which is already given \
          [test]"]
    );
    let errors = Config::parse(contents, LANGUAGES).unwrap_err();
    let report =
        LoadError::Invalid { path: "zenith.toml".into(), contents: contents.to_string(), errors };
    let expected = [
        "error: `languages.test` is another name for `languages.testing`, which is already given",
        " --> zenith.toml:4:12",
        "  |",
        "4 | [languages.test]",
        "  |            ^^^^",
        "  |",
        "  = help: give the language one table",
    ];
    assert_eq!(report.to_string(), expected.join("\n"));
}

#[test]
fn an_empty_languages_table_enables_no_language() {
    let contents = include_str!("fixtures/languages-empty.toml");
    assert_eq!(invalid(contents), ["1:1: no language is enabled [[languages]]"]);
    let error = &Config::parse(contents, LANGUAGES).unwrap_err()[0];
    assert_eq!(error.kind, ConfigErrorKind::NoLanguages { example: Some("test".into()) });
    assert_eq!(error.kind.to_string(), "no language is enabled");
}

#[test]
fn unknown_languages_are_errors_with_suggestions() {
    let contents = include_str!("fixtures/languages-unknown.toml");
    assert_eq!(
        invalid(contents),
        [
            "3:12: unknown language `testng`; did you mean `testing`? [testng]",
            "5:12: unknown language `go`, expected one of `test`, `testing`, `other` [go]",
        ]
    );
    // The language that is written is still enabled.
    let errors = Config::parse(contents, LANGUAGES).unwrap_err();
    assert_eq!(errors.len(), 2);
}

#[test]
fn the_tables_of_the_old_shape_are_unknown_keys() {
    assert_eq!(
        invalid(include_str!("fixtures/removed-tables.toml")),
        [
            "2:1: unknown key `include`; did you mean `exclude`? [include]",
            "4:2: unknown key `resolve`, expected one of `files`, `languages`, `analysis`, \
             `baseline`, `cache`, `contracts` [resolve]",
        ]
    );
}

#[test]
fn schemes_of_an_enabled_language_are_allowed() {
    let config = valid(include_str!("fixtures/languages-with-schemes.toml"));
    let ContractKind::Forbidden(forbidden) = &config.contracts[0].kind else {
        panic!("expected `forbidden`");
    };
    let schemes: Vec<_> = forbidden.to.iter().map(Selector::scheme).collect();
    assert_eq!(schemes, [Some("ext"), Some("core"), Some("sys"), Some("pkg")]);
}

#[test]
fn schemes_of_a_disabled_language_are_errors() {
    let contents = include_str!("fixtures/languages-empty-with-schemes.toml");
    // Selectors, `ignore_imports` entries and layers are all checked, and
    // `pkg:` belongs to no language.
    let errors = invalid(contents);
    let scheme = |selector: &str, text: &str| {
        let (name, _) = selector.split_once(':').unwrap();
        format!(
            "`{selector}` uses the `{name}:` scheme of the `test` language, which is not enabled \
             [{text}]"
        )
    };
    assert_eq!(
        errors,
        [
            "1:1: no language is enabled [[languages]]".to_string(),
            format!("6:7: in contract `no-sharp`: {}", scheme("ext:sharp", "\"ext:sharp\"")),
            format!("6:20: in contract `no-sharp`: {}", scheme("core:*", "\"core:*\"")),
            format!(
                "7:19: in contract `no-sharp`: {}",
                scheme("sys:sqlite", "\"web/src -> sys:sqlite\"")
            ),
            format!(
                "11:11: in contract `tiers`: {}",
                scheme("ext:react", "\"web/src | ext:react\"")
            ),
        ]
    );

    // The report says how to enable the language.
    let errors = Config::parse(contents, LANGUAGES).unwrap_err();
    let error =
        LoadError::Invalid { path: "zenith.toml".into(), contents: contents.into(), errors };
    let expected = [
        "error: `ext:sharp` uses the `ext:` scheme of the `test` language, which is not enabled",
        " --> zenith.toml:6:7",
        "  |",
        "6 | to = [\"ext:sharp\", \"core:*\", \"pkg:@acme/server\"]",
        "  |       ^^^^^^^^^^^",
        "  |",
        "  = note: in contract `no-sharp`",
        "  = help: enable it with a `[languages.test]` table",
    ];
    assert!(error.to_string().contains(&expected.join("\n")), "{error}");
}

#[test]
fn unknown_schemes_are_errors_with_suggestions() {
    assert_eq!(
        invalid(include_str!("fixtures/unknown-scheme.toml")),
        [
            "4:7: in contract `a`: invalid selector `exy:react`: unknown scheme `exy:`; did you \
             mean `ext:`? [\"exy:react\"]",
            "4:20: in contract `a`: invalid selector `gopkg:fmt`: unknown scheme `gopkg:`, \
             expected one of `pkg:`, `ext:`, `core:`, `sys:`, `oth:` [\"gopkg:fmt\"]",
        ]
    );
}

#[test]
fn parses_a_forbidden_contract() {
    let config = valid(
        r#"
[contracts.client-safety]
type = "forbidden"
name = "Client components never reach server-only code"
from = [{ path = "web/src/**", directive = "use client", except = ["web/src/legacy"] }]
to = ["ext:sharp", "core:*", "sys:sqlite", "pkg:@acme/server"]
allow_indirect_imports = true
severity = "warn"
exclude_kinds = ["type"]
ignore_imports = ["web/src/lib/** -> ext:sharp"]
unmatched_ignore_imports = "warn"
guidance = "Keep server-only modules out of client bundles."
"#,
    );

    let contract = &config.contracts[0];
    assert_eq!(contract.id, "client-safety");
    assert_eq!(contract.name, "Client components never reach server-only code");
    assert_eq!(contract.severity, Severity::Warn);
    assert_eq!(contract.exclude_kinds, KindSet::TYPE);
    assert_eq!(contract.ignore_imports.len(), 1);
    assert_eq!(contract.ignore_imports[0].importer().as_str(), "web/src/lib/**");
    assert_eq!(contract.ignore_imports[0].imported().as_str(), "ext:sharp");
    assert_eq!(contract.unmatched_ignore_imports, AlertLevel::Warn);
    assert_eq!(
        contract.guidance.as_deref(),
        Some("Keep server-only modules out of client bundles.")
    );
    assert_eq!(contract.kind.type_name(), "forbidden");

    let ContractKind::Forbidden(forbidden) = &contract.kind else { panic!("expected `forbidden`") };
    assert!(forbidden.allow_indirect_imports);
    let from = &forbidden.from[0];
    assert_eq!(from.as_str(), "web/src/**");
    assert_eq!(from.directive(), Some("use client"));
    assert_eq!(selectors(from.except()), ["web/src/legacy"]);
    assert_eq!(selectors(&forbidden.to), ["ext:sharp", "core:*", "sys:sqlite", "pkg:@acme/server"]);
    assert!(matches!(forbidden.to[0].target(), SelectorTarget::Scheme { scheme: "ext", .. }));
    assert!(matches!(forbidden.to[2].target(), SelectorTarget::Scheme { scheme: "sys", .. }));
    assert!(matches!(forbidden.to[3].target(), SelectorTarget::Package(_)));
}

#[test]
fn parses_an_independence_contract() {
    let config = valid(
        r#"
[contracts.features-independent]
type = "independence"
modules = ["web/src/features/*"]
ignore_imports = ["web/src/features/* -> web/src/features/shared"]
boundaries = ["web/src/server", { path = "web/src/**", directive = "use server" }]
"#,
    );
    let ContractKind::Independence(independence) = &config.contracts[0].kind else {
        panic!("expected `independence`")
    };
    assert_eq!(selectors(&independence.modules), ["web/src/features/*"]);
    assert!(!independence.allow_indirect_imports);
    assert_eq!(selectors(&independence.boundaries), ["web/src/server", "web/src/**"]);
    assert_eq!(independence.boundaries[1].directive(), Some("use server"));
}

#[test]
fn parses_a_layers_contract() {
    let config = valid(
        r#"
[contracts.package-tiers]
type = "layers"
layers = [
  "pkg:web | pkg:sdk | apps/* | services/*",
  { layer = "pkg:@acme/ui", closed = true },
  "pkg:@acme/types : (pkg:@acme/config)",
]
allow_indirect_imports = true

[contracts.domain-internals]
type = "layers"
containers = ["packages/api/src/domains/*"]
layers = ["hooks", "api", "(types)"]
exhaustive = true
exhaustive_ignores = ["index", "*.test"]
"#,
    );

    let ContractKind::Layers(tiers) = &config.contracts[0].kind else {
        panic!("expected `layers`")
    };
    assert_eq!(tiers.layers.len(), 3);
    assert_eq!(tiers.layers[0].members.len(), 4);
    assert!(tiers.layers[0].independent);
    assert!(!tiers.layers[0].closed);
    assert!(tiers.layers[1].closed);
    assert!(!tiers.layers[2].independent);
    assert!(tiers.layers[2].members[1].optional);
    assert!(tiers.containers.is_empty());
    assert!(!tiers.exhaustive);
    assert!(tiers.allow_indirect_imports);

    let ContractKind::Layers(domains) = &config.contracts[1].kind else {
        panic!("expected `layers`")
    };
    assert_eq!(selectors(&domains.containers), ["packages/api/src/domains/*"]);
    assert!(domains.exhaustive);
    assert_eq!(
        domains.exhaustive_ignores,
        [NamePattern::new("index").unwrap(), NamePattern::new("*.test").unwrap()]
    );
    assert!(domains.layers[2].members[0].optional);
    assert_eq!(domains.layers[2].members[0].selector.as_str(), "types");
}

#[test]
fn parses_a_protected_contract() {
    let config = valid(
        r#"
[contracts.agents-are-api-only]
type = "protected"
modules = ["web/src/agents"]
allowed_importers = ["web/src/app/api"]
"#,
    );
    let ContractKind::Protected(protected) = &config.contracts[0].kind else {
        panic!("expected `protected`")
    };
    assert_eq!(selectors(&protected.modules), ["web/src/agents"]);
    assert_eq!(selectors(&protected.allowed_importers), ["web/src/app/api"]);
}

#[test]
fn parses_an_acyclic_contract() {
    let config = valid(
        r#"
[contracts.packages-acyclic]
type = "acyclic"
modules = ["pkg:*"]
depth = 2
skip = ["pkg:@acme/legacy"]
"#,
    );
    let ContractKind::Acyclic(acyclic) = &config.contracts[0].kind else {
        panic!("expected `acyclic`")
    };
    assert_eq!(selectors(&acyclic.modules), ["pkg:*"]);
    assert_eq!(acyclic.depth, 2);
    assert_eq!(selectors(&acyclic.skip), ["pkg:@acme/legacy"]);
}

#[test]
fn parses_an_encapsulation_contract() {
    let config = valid(
        r#"
[contracts.ui-public-surface]
type = "encapsulation"
modules = ["pkg:@acme/ui"]
entrypoints = ["index", "src/components/*/index"]
allowed_importers = ["**/*.test.ts"]
"#,
    );
    let ContractKind::Encapsulation(encapsulation) = &config.contracts[0].kind else {
        panic!("expected `encapsulation`")
    };
    assert_eq!(selectors(&encapsulation.modules), ["pkg:@acme/ui"]);
    assert_eq!(
        encapsulation.entrypoints,
        [Glob::new("index").unwrap(), Glob::new("src/components/*/index").unwrap()]
    );
    assert_eq!(selectors(&encapsulation.allowed_importers), ["**/*.test.ts"]);
}

#[test]
fn parses_a_limits_contract() {
    let config = valid(include_str!("fixtures/limits.toml"));
    let ContractKind::Limits(both) = &config.contracts[0].kind else { panic!("expected `limits`") };
    assert_eq!(selectors(&both.modules), ["web/src/*"]);
    assert_eq!((both.max_fan_in, both.max_fan_out), (Some(20), Some(12)));
    let ContractKind::Limits(one) = &config.contracts[1].kind else { panic!("expected `limits`") };
    assert_eq!((one.max_fan_in, one.max_fan_out), (None, Some(0)));
}

#[test]
fn a_limits_contract_must_set_a_limit() {
    assert_eq!(
        only_error(include_str!("fixtures/limits-without-limits.toml")),
        "1:12: in contract `l`: `limits` contracts must set `max_fan_in`, `max_fan_out` or both \
         [l]"
    );
}

#[test]
fn contracts_have_defaults() {
    let config = valid(
        r#"
[analysis]
exclude_kinds = ["type"]

[contracts.a]
type = "forbidden"
from = ["a"]
to = ["b"]

[contracts.b]
type = "encapsulation"
modules = ["pkg:*"]

[contracts.c]
type = "acyclic"
modules = ["pkg:*"]

[contracts.d]
type = "layers"
layers = ["a"]
"#,
    );

    for contract in &config.contracts {
        assert_eq!(contract.name, contract.id);
        assert_eq!(contract.severity, Severity::Error);
        assert_eq!(contract.exclude_kinds, KindSet::TYPE);
        assert!(contract.ignore_imports.is_empty());
        assert_eq!(contract.unmatched_ignore_imports, AlertLevel::Error);
        assert_eq!(contract.guidance, None);
    }
    let ContractKind::Forbidden(forbidden) = &config.contracts[0].kind else { panic!() };
    assert!(!forbidden.allow_indirect_imports);
    assert!(forbidden.boundaries.is_empty());
    let ContractKind::Encapsulation(encapsulation) = &config.contracts[1].kind else { panic!() };
    assert_eq!(encapsulation.entrypoints, [Glob::new("index").unwrap()]);
    assert!(encapsulation.allowed_importers.is_empty());
    let ContractKind::Acyclic(acyclic) = &config.contracts[2].kind else { panic!() };
    assert_eq!(acyclic.depth, 0);
    assert!(acyclic.skip.is_empty());
    let ContractKind::Layers(layers) = &config.contracts[3].kind else { panic!() };
    assert!(layers.containers.is_empty());
    assert!(!layers.exhaustive);
    assert!(layers.exhaustive_ignores.is_empty());
    assert!(!layers.allow_indirect_imports);
    assert!(layers.boundaries.is_empty());
}

#[test]
fn a_contract_can_clear_the_excluded_kinds() {
    let config = valid(
        r#"
[analysis]
exclude_kinds = ["type"]

[contracts.all-kinds]
type = "acyclic"
modules = ["pkg:*"]
exclude_kinds = []
"#,
    );
    assert!(config.contracts[0].exclude_kinds.is_empty());
}

#[test]
fn contracts_keep_the_order_of_the_file() {
    let config = valid(
        r#"
[contracts.zebra]
type = "acyclic"
modules = ["a"]

[contracts.apple]
type = "acyclic"
modules = ["a"]

[contracts.mango_2]
type = "acyclic"
modules = ["a"]
"#,
    );
    let ids: Vec<_> = config.contracts.iter().map(|contract| contract.id.as_str()).collect();
    assert_eq!(ids, ["zebra", "apple", "mango_2"]);
    assert_eq!(config.contract("apple").map(|contract| contract.id.as_str()), Some("apple"));
    assert!(config.contract("pear").is_none());
}

#[test]
fn unknown_keys_are_errors_everywhere() {
    for (contents, expected) in [
        ("filez = 1\n", "1:1: unknown key `filez`; did you mean `files`? [filez]"),
        ("[contract.a]\n", "1:2: unknown key `contract`; did you mean `contracts`? [contract]"),
        ("[files]\nexclud = []\n", "2:1: unknown key `exclud`; did you mean `exclude`? [exclud]"),
        ("[languages.test]\nname = []\n", "2:1: unknown key `name`; did you mean `names`? [name]"),
        (
            "[analysis]\nexclude_kind = []\n",
            "2:1: unknown key `exclude_kind`; did you mean `exclude_kinds`? [exclude_kind]",
        ),
        ("[baseline]\npth = \"a\"\n", "2:1: unknown key `pth`; did you mean `path`? [pth]"),
        (
            "[cache]\nenable = false\n",
            "2:1: unknown key `enable`; did you mean `enabled`? [enable]",
        ),
        (
            "[languages.test]\nzzz = 1\n",
            "2:1: unknown key `zzz`, expected `include` or `names` [zzz]",
        ),
        (
            "[languages]\ntest = {}\ngo = {}\n",
            "3:1: unknown language `go`, expected one of `test`, `testing`, `other` [go]",
        ),
    ] {
        assert_eq!(only_error(contents), expected, "{contents}");
    }
}

#[test]
fn unknown_keys_of_contracts_are_errors() {
    for (contract, expected) in [
        (
            "type = \"forbidden\"\nfromm = [\"a\"]\nto = []",
            "3:1: in contract `c`: unknown key `fromm`; did you mean `from`? [fromm]",
        ),
        (
            "type = \"independence\"\nmodule = [\"a\"]",
            "3:1: in contract `c`: unknown key `module`; did you mean `modules`? [module]",
        ),
        (
            "type = \"layers\"\nlayers = [\"a\"]\nexhaustiv = true",
            "4:1: in contract `c`: unknown key `exhaustiv`; did you mean `exhaustive`? [exhaustiv]",
        ),
        (
            "type = \"protected\"\nmodules = []\nallowed_importer = []",
            "4:1: in contract `c`: unknown key `allowed_importer`; did you mean `allowed_importers`? \
             [allowed_importer]",
        ),
        (
            "type = \"acyclic\"\nmodules = []\ndept = 1",
            "4:1: in contract `c`: unknown key `dept`; did you mean `depth`? [dept]",
        ),
        (
            "type = \"encapsulation\"\nmodules = []\nentrypoint = []",
            "4:1: in contract `c`: unknown key `entrypoint`; did you mean `entrypoints`? [entrypoint]",
        ),
        (
            "type = \"acyclic\"\nmodules = []\nsevrity = \"warn\"",
            "4:1: in contract `c`: unknown key `sevrity`; did you mean `severity`? [sevrity]",
        ),
        (
            "type = \"forbidden\"\nfrom = [{ paths = \"a\" }]\nto = []",
            "3:11: in contract `c`: unknown key `paths`; did you mean `path`? [paths]",
        ),
        (
            "type = \"layers\"\nlayers = [{ layer = \"a\", close = true }]",
            "3:26: in contract `c`: unknown key `close`; did you mean `closed`? [close]",
        ),
        // A key of another contract type.
        (
            "type = \"forbidden\"\nfrom = []\nto = []\nmodules = []",
            "5:1: in contract `c`: unknown key `modules`, expected one of `from`, `to`, \
             `allow_indirect_imports`, `boundaries`, `name`, `severity`, `exclude_kinds`, `reexports`, \
             `ignore_imports`, `unmatched_ignore_imports`, `guidance` [modules]",
        ),
    ] {
        let contents = format!("[contracts.c]\n{contract}\n");
        assert_eq!(only_error(&contents), expected, "{contents}");
    }
}

#[test]
fn unknown_values_are_errors() {
    for (contents, expected) in [
        (
            "[contracts.c]\ntype = \"layer\"\n",
            "2:8: in contract `c`: unknown contract type `layer`; did you mean `layers`? [\"layer\"]",
        ),
        (
            "[contracts.c]\ntype = \"cycles\"\n",
            "2:8: in contract `c`: unknown contract type `cycles`; did you mean `acyclic`? [\"cycles\"]",
        ),
        (
            "[contracts.c]\ntype = \"graph\"\n",
            "2:8: in contract `c`: unknown contract type `graph`, expected one of `forbidden`, \
             `independence`, `layers`, `protected`, `acyclic`, `encapsulation`, `limits` \
             [\"graph\"]",
        ),
        (
            "[analysis]\nexclude_kinds = [\"types\"]\n",
            "2:18: unknown value `types`; did you mean `type`? [\"types\"]",
        ),
        (
            "[baseline]\nstale = \"warning\"\n",
            "2:9: unknown value `warning`; did you mean `warn`? [\"warning\"]",
        ),
        (
            "[contracts.c]\ntype = \"acyclic\"\nmodules = []\nseverity = \"fatal\"\n",
            "4:12: in contract `c`: unknown value `fatal`, expected `error` or `warn` [\"fatal\"]",
        ),
        (
            "[contracts.c]\ntype = \"acyclic\"\nmodules = []\nunmatched_ignore_imports = \"off\"\n",
            "4:28: in contract `c`: unknown value `off`, expected one of `error`, `warn`, `none` \
             [\"off\"]",
        ),
    ] {
        assert_eq!(only_error(contents), expected, "{contents}");
    }
}

#[test]
fn values_of_the_wrong_type_are_errors() {
    for (contents, expected) in [
        ("files = 1\n", "1:9: invalid type: integer `1`, expected a table [1]"),
        (
            "[files]\nexclude = \"a\"\n",
            "2:11: invalid type: string \"a\", expected an array [\"a\"]",
        ),
        (
            "[files]\nrespect_gitignore = \"yes\"\n",
            "2:21: invalid type: string \"yes\", expected a boolean [\"yes\"]",
        ),
        (
            "[languages.test]\nnames = \"a\"\n",
            "2:9: invalid type: string \"a\", expected an array [\"a\"]",
        ),
        ("contracts = 1\n", "1:13: `contracts` must be a table [1]"),
        ("[contracts]\nc = \"forbidden\"\n", "2:5: `contracts.c` must be a table [\"forbidden\"]"),
        ("[contracts.c]\nfrom = []\n", "1:1: in contract `c`: missing key `type` [[contracts.c]]"),
        ("[contracts.c]\ntype = 1\n", "2:8: in contract `c`: `type` must be a string [1]"),
        (
            "[contracts.c]\ntype = \"forbidden\"\nfrom = []\n",
            "1:1: in contract `c`: missing key `to` [[contracts.c]]",
        ),
        (
            "[contracts.c]\ntype = \"forbidden\"\nfrom = [1]\nto = []\n",
            "3:9: in contract `c`: invalid type: integer `1`, expected a selector string or table [1]",
        ),
        (
            "[contracts.c]\ntype = \"forbidden\"\nfrom = \"a\"\nto = []\n",
            "3:8: in contract `c`: invalid type: string \"a\", expected an array [\"a\"]",
        ),
        (
            "[contracts.c]\ntype = \"layers\"\nlayers = [[\"a\"]]\n",
            "3:11: in contract `c`: invalid type: array, expected a layer string or table [[\"a\"]]",
        ),
        (
            "[contracts.c]\ntype = \"acyclic\"\nmodules = []\ndepth = -1\n",
            "4:9: in contract `c`: invalid value: integer `-1`, expected u32 [-1]",
        ),
    ] {
        assert_eq!(only_error(contents), expected, "{contents}");
    }
}

#[test]
fn syntax_errors_have_a_location() {
    assert_eq!(only_error("[files\n"), "1:7: invalid TOML: unclosed table, expected `]` []");
    let contents = "[files]\n[files]\n";
    let errors = Config::parse(contents, LANGUAGES).unwrap_err();
    assert!(matches!(errors[0].kind, ConfigErrorKind::Syntax { .. }), "{errors:?}");
    assert_eq!(errors[0].range.map(|range| line_column(contents, range).0), Some(2));
}

#[test]
fn contract_ids_may_only_have_letters_digits_dashes_and_underscores() {
    valid("[contracts.Web_Layers-2]\ntype = \"acyclic\"\nmodules = [\"a\"]\n");
    for id in ["web layers", "web.layers", "", "é"] {
        let contents = format!("[contracts.\"{id}\"]\ntype = \"acyclic\"\nmodules = [\"a\"]\n");
        let errors = Config::parse(&contents, LANGUAGES).unwrap_err();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].kind, ConfigErrorKind::InvalidContractId { id: id.to_string() });
        assert_eq!(errors[0].contract, None);
    }
}

#[test]
fn invalid_selectors_are_errors() {
    for (contract, expected) in [
        (
            "type = \"forbidden\"\nfrom = [\"/web\"]\nto = []",
            "3:9: in contract `c`: invalid selector `/web`: leading `/`; globs are relative to the \
             project root [\"/web\"]",
        ),
        (
            "type = \"forbidden\"\nfrom = []\nto = [\"ext:\"]",
            "4:7: in contract `c`: invalid selector `ext:`: the pattern is empty [\"ext:\"]",
        ),
        (
            "type = \"independence\"\nmodules = [\"web/../api\"]",
            "3:12: in contract `c`: invalid selector `web/../api`: `..` is not allowed; paths are \
             relative to the project root [\"web/../api\"]",
        ),
        (
            "type = \"protected\"\nmodules = [\"a\"]\nallowed_importers = [{ path = \"b/{c\" }]",
            "4:31: in contract `c`: invalid selector `b/{c`: unclosed `{` [\"b/{c\"]",
        ),
        (
            "type = \"acyclic\"\nmodules = [{ path = \"a\", except = [\"a//b\"] }]",
            "3:36: in contract `c`: invalid selector `a//b`: empty path segment [\"a//b\"]",
        ),
        (
            "type = \"forbidden\"\nfrom = [{ path = \"ext:react\", directive = \"use client\" }]\nto = []",
            "3:43: in contract `c`: invalid selector `ext:react`: `directive` only applies to path \
             and `pkg:` selectors, which match files [\"use client\"]",
        ),
        (
            "type = \"layers\"\nlayers = [\"a\"]\ncontainers = [\"a**\"]",
            "4:15: in contract `c`: invalid selector `a**`: `**` must be a whole path segment; use \
             `*` within a segment [\"a**\"]",
        ),
        (
            "type = \"acyclic\"\nmodules = [\"a\"]\nskip = [\"\"]",
            "4:9: in contract `c`: invalid selector ``: the pattern is empty [\"\"]",
        ),
        (
            "type = \"encapsulation\"\nmodules = [\"pkg:\"]",
            "3:12: in contract `c`: invalid selector `pkg:`: the pattern is empty [\"pkg:\"]",
        ),
    ] {
        let contents = format!("[contracts.c]\n{contract}\n");
        assert_eq!(only_error(&contents), expected, "{contents}");
    }
}

#[test]
fn invalid_globs_and_name_patterns_are_errors() {
    for (contents, expected) in [
        (
            "[languages.test]\ninclude = [\"/src\"]\n",
            "2:12: invalid pattern `/src`: leading `/`; globs are relative to the project root [\"/src\"]",
        ),
        ("[files]\nexclude = [\"a/\"]\n", "2:12: invalid pattern `a/`: trailing `/` [\"a/\"]"),
        (
            "[languages.test]\nnames = [\"virtual:{\"]\n",
            "2:10: invalid pattern `virtual:{`: unclosed `{` [\"virtual:{\"]",
        ),
        (
            "[contracts.c]\ntype = \"layers\"\nlayers = [\"a\"]\ncontainers = [\"x\"]\nexhaustive = true\nexhaustive_ignores = [\"\\\\\"]\n",
            "6:23: in contract `c`: invalid pattern `\\`: trailing `\\` [\"\\\\\"]",
        ),
        (
            "[contracts.c]\ntype = \"encapsulation\"\nmodules = [\"a\"]\nentrypoints = [\"src//index\"]\n",
            "4:16: in contract `c`: invalid pattern `src//index`: empty path segment [\"src//index\"]",
        ),
    ] {
        assert_eq!(only_error(contents), expected, "{contents}");
    }
}

#[test]
fn invalid_layers_are_errors() {
    for (contract, expected) in [
        (
            "layers = [\"a | b : c\"]",
            "3:11: in contract `c`: invalid layer `a | b : c`: a layer cannot mix `|` and `:` \
             separators [\"a | b : c\"]",
        ),
        (
            "layers = [\"a |\"]",
            "3:11: in contract `c`: invalid layer `a |`: a member is empty [\"a |\"]",
        ),
        (
            "layers = [{ layer = \"a | /b\" }]",
            "3:21: in contract `c`: invalid layer `a | /b`: invalid selector `/b`: leading `/`; globs \
             are relative to the project root [\"a | /b\"]",
        ),
        (
            "containers = [\"apps/*\"]\nlayers = [\"ui\", \"pkg:@acme/core\"]",
            "4:17: in contract `c`: invalid layer `pkg:@acme/core`: `pkg:@acme/core` is not a path; \
             with `containers`, layers are paths relative to each container [\"pkg:@acme/core\"]",
        ),
        ("layers = []", "3:10: in contract `c`: `layers` must list at least one layer [[]]"),
        (
            "layers = [\"a\"]\nexhaustive = true",
            "4:14: in contract `c`: `exhaustive` requires `containers` [true]",
        ),
    ] {
        let contents = format!("[contracts.c]\ntype = \"layers\"\n{contract}\n");
        assert_eq!(only_error(&contents), expected, "{contents}");
    }
}

#[test]
fn boundaries_without_chains_are_an_error() {
    for contents in [
        include_str!("fixtures/boundaries-with-indirect-imports-forbidden.toml"),
        include_str!("fixtures/boundaries-with-indirect-imports-independence.toml"),
        include_str!("fixtures/boundaries-with-indirect-imports-layers.toml"),
    ] {
        let line = contents.lines().count();
        let expected = format!(
            "{line}:14: in contract `c`: `boundaries` has no effect with \
             `allow_indirect_imports = true`, since direct imports pass through nothing [[\"a\"]]"
        );
        assert_eq!(only_error(contents), expected, "{contents}");
    }
}

#[test]
fn reexports_default_to_the_analysis_and_encapsulation_never_sees_through_them() {
    assert_eq!(Config::defaults(LANGUAGES).analysis.reexports, Reexports::Opaque);
    let config = valid(include_str!("fixtures/reexports.toml"));
    let reexports: Vec<_> = config
        .contracts
        .iter()
        .map(|contract| (contract.id.as_str(), contract.reexports))
        .collect();
    assert_eq!(
        reexports,
        [("a", Reexports::Transparent), ("b", Reexports::Opaque), ("c", Reexports::Opaque)]
    );
}

#[test]
fn contracts_parsed_for_a_config_default_to_its_analysis() {
    let config = valid(include_str!("fixtures/analysis-defaults.toml"));
    let contracts =
        config.parse_contracts(include_str!("fixtures/contracts-only.toml"), LANGUAGES).unwrap();
    let defaults: Vec<_> = contracts
        .iter()
        .map(|contract| (contract.id.as_str(), contract.exclude_kinds, contract.reexports))
        .collect();
    assert_eq!(
        defaults,
        [
            ("inherits", KindSet::TYPE, Reexports::Transparent),
            ("overrides", KindSet::empty(), Reexports::Opaque),
        ]
    );
}

#[test]
fn contracts_parsed_for_a_config_have_only_contracts_and_its_languages() {
    let contents = include_str!("fixtures/contracts-with-a-table.toml");
    let errors = match Config::defaults(LANGUAGES).parse_contracts(contents, LANGUAGES) {
        Ok(contracts) => panic!("expected errors, got {contracts:?}"),
        Err(errors) => render(contents, &errors),
    };
    assert_eq!(
        errors,
        [
            "1:2: unknown key `files`, expected `contracts` [files]",
            "7:7: in contract `other-scheme`: `oth:x` uses the `oth:` scheme of the `other` \
             language, which is not enabled [\"oth:x\"]",
        ]
    );
}

#[test]
fn encapsulation_that_sees_through_reexports_is_an_error() {
    let contents = include_str!("fixtures/reexports-on-encapsulation.toml");
    assert_eq!(
        only_error(contents),
        "4:13: in contract `c`: `encapsulation` contracts can't see through re-exports, since \
         importing through an entry point, which often only re-exports, is what they allow \
         [\"transparent\"]"
    );
}

#[test]
fn every_error_is_reported_in_the_order_of_the_file() {
    let contents = r#"
[contracts.b]
type = "forbidden"
from = ["/a", "web/src"]
to = ["ext:"]
ignore_imports = ["oops"]

[files]
exclude = ["a//b"]

[contracts.a]
type = "acyclic"
modules = []
dept = 1

[contracts.c]
type = "layer"

[languages.test]
name = []

[baseline]
stale = "warning"

[filez]
"#;
    assert_eq!(
        invalid(contents),
        [
            "4:9: in contract `b`: invalid selector `/a`: leading `/`; globs are relative to the \
             project root [\"/a\"]",
            "5:7: in contract `b`: invalid selector `ext:`: the pattern is empty [\"ext:\"]",
            "6:19: in contract `b`: invalid `ignore_imports` entry `oops`: expected `<importer> -> \
             <imported>` [\"oops\"]",
            "9:12: invalid pattern `a//b`: empty path segment [\"a//b\"]",
            "14:1: in contract `a`: unknown key `dept`; did you mean `depth`? [dept]",
            "17:8: in contract `c`: unknown contract type `layer`; did you mean `layers`? [\"layer\"]",
            "20:1: unknown key `name`; did you mean `names`? [name]",
            "23:9: unknown value `warning`; did you mean `warn`? [\"warning\"]",
            "25:2: unknown key `filez`; did you mean `files`? [filez]",
        ]
    );
}

#[test]
fn errors_are_structured() {
    let errors =
        Config::parse("[contracts.c]\ntype = \"forbidden\"\nfrom = [\"/a\"]\nto = []\n", LANGUAGES)
            .unwrap_err();
    assert_eq!(
        errors,
        [ConfigError {
            kind: ConfigErrorKind::InvalidSelector {
                selector: "/a".into(),
                error: SelectorError::Pattern(GlobError::LeadingSlash),
            },
            contract: Some(InContract("c".into())),
            range: Some(ByteRange::new(41, 45)),
        }]
    );
}

#[test]
fn selectors_layers_and_ignored_imports_keep_their_range() {
    let contents = r#"
[contracts.c]
type = "layers"
containers = [{ path = "apps/*", except = ["apps/legacy"] }]
layers = ["ui | (api)"]
ignore_imports = ["ui -> api"]
"#;
    let config = valid(contents);
    let contract = &config.contracts[0];
    let ContractKind::Layers(layers) = &contract.kind else { panic!("expected `layers`") };
    let at = |range: Option<ByteRange>| {
        let range = range.expect("a range");
        let (line, column) = line_column(contents, range);
        (line, column, &contents[Range::from(range)])
    };

    let container = &layers.containers[0];
    assert_eq!(at(container.range()), (4, 24, "\"apps/*\""));
    assert_eq!(at(container.except()[0].range()), (4, 44, "\"apps/legacy\""));
    let layer = &layers.layers[0];
    assert_eq!(at(layer.range), (5, 11, "\"ui | (api)\""));
    for member in &layer.members {
        assert_eq!(member.selector.range(), layer.range);
    }
    let ignore = &contract.ignore_imports[0];
    assert_eq!(at(ignore.range()), (6, 19, "\"ui -> api\""));
    assert_eq!(ignore.importer().range(), ignore.range());
    assert_eq!(ignore.imported().range(), ignore.range());

    // Selectors from elsewhere, such as the command line, have none.
    assert_eq!(Selector::parse("ui", LANGUAGES).map(|selector| selector.range()), Ok(None));
}
