//! Parsing the `[languages.terraform]` table: its options, its defaults, its
//! errors, and the selector schemes it enables.

use zenith_config::{Config, ContractKind, Languages, Selector};
use zenith_utils::glob::{Glob, NamePattern};

use super::*;

/// The languages of the tests: only Terraform.
const LANGUAGES: Languages = Languages::new(&[&TERRAFORM]);

/// The options of Terraform in a configuration that must be valid.
fn terraform(contents: &str) -> TerraformConfig {
    let config = Config::parse(contents, LANGUAGES).unwrap_or_else(|errors| panic!("{errors:?}"));
    config.languages.options::<TerraformConfig>().expect("terraform is enabled").clone()
}

/// The only error of an invalid configuration.
fn only_error(contents: &str) -> String {
    let errors = Config::parse(contents, LANGUAGES).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:#?}");
    errors[0].to_string()
}

#[test]
fn terraform_is_enabled_by_its_table_alone() {
    assert!(Config::defaults(LANGUAGES).languages.options::<TerraformConfig>().is_none());
    let terraform = terraform("[languages.terraform]\n");
    assert_eq!((terraform.include, terraform.ignore_unresolved), (None, vec![]));
}

#[test]
fn parses_the_terraform_table() {
    let terraform = terraform(include_str!("fixtures/terraform-table.toml"));
    assert_eq!(
        terraform.include,
        Some(vec![
            Glob::new("infra/**/*.tf").unwrap(),
            Glob::new("live/**/terragrunt.hcl").unwrap()
        ])
    );
    assert_eq!(terraform.ignore_unresolved, [NamePattern::new("../../shared/*").unwrap()]);
}

#[test]
fn invalid_tables_are_errors() {
    for (contents, expected) in [
        (
            include_str!("fixtures/unknown-key.toml"),
            "unknown key `zzz`, expected `include` or `ignore_unresolved`",
        ),
        (include_str!("fixtures/invalid-pattern.toml"), "invalid pattern `infra/{`: unclosed `{`"),
    ] {
        assert_eq!(only_error(contents), expected, "{contents}");
    }
}

#[test]
fn terraform_enables_the_tfmod_scheme() {
    let config = Config::parse(include_str!("fixtures/schemes.toml"), LANGUAGES).unwrap();
    let ContractKind::Forbidden(forbidden) = &config.contracts[0].kind else {
        panic!("expected `forbidden`");
    };
    let schemes: Vec<_> = forbidden.to.iter().map(Selector::scheme).collect();
    assert_eq!(schemes, [Some("tfmod"), Some("pkg")]);
}

#[test]
fn terragrunt_files_are_scanned_by_name() {
    let config = Config::parse("[languages.terraform]\n", LANGUAGES).unwrap();
    let scanned = config.languages.scanned();
    assert_eq!(
        (scanned.extensions, scanned.file_names),
        (vec!["tf", "tofu"], vec!["terragrunt.hcl"])
    );
    for name in ["main.tf", "live/prod/main.tofu", "live/prod/terragrunt.hcl"] {
        assert_eq!(config.languages.language_of(name), Some(0), "{name}");
    }
    for name in ["live/root.hcl", ".terraform.lock.hcl", "main.tf.json"] {
        assert_eq!(config.languages.language_of(name), None, "{name}");
    }
}
