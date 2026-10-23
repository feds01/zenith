//! Terraform in the configuration: the [`TERRAFORM`] language, the
//! `[languages.terraform]` table, and the options it is parsed into.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use zenith_config::{Language, LanguageOptions, LanguageTable, Located};
use zenith_lang::{Backend, BackendSettings};
use zenith_utils::glob::{Glob, NamePattern};

use crate::backend::TerraformBackend;

/// Terraform, with OpenTofu and Terragrunt.
pub const TERRAFORM: Language = Language {
    name: "terraform",
    table_names: &["terraform"],
    description: "Terraform, OpenTofu and Terragrunt: how their files are found and their module \
                  sources resolved.",
    enabled_by_default: false,
    include: &["**/*.tf", "**/*.tofu", "**/terragrunt.hcl"],
    source_extensions: &["tf", "tofu"],
    source_file_names: &["terragrunt.hcl"],
    optional_extensions: &["tf", "tofu"],
    // `tfmod:` names the modules of registries and repositories.
    schemes: &["tfmod"],
    parse: |table| Arc::new(TerraformConfig::parse(table)),
    defaults: || Arc::new(TerraformConfig::default()),
    schema: |generator| generator.subschema_for::<TerraformTable>(),
};

/// The options of Terraform. `None` options leave the defaults in place.
#[derive(Debug, Clone, Default)]
pub struct TerraformConfig {
    /// Globs of the files to scan. `None` scans every file with one of the
    /// [`TERRAFORM`] source extensions or file names.
    pub include: Option<Vec<Glob>>,

    /// Patterns of module sources and dependency paths that are expected not
    /// to resolve, so are not reported as unresolved.
    pub ignore_unresolved: Vec<NamePattern>,
}

impl TerraformConfig {
    /// Parse the table of Terraform. An invalid table has the defaults, so
    /// that the language is still enabled.
    fn parse(table: &mut LanguageTable<'_, '_>) -> Self {
        let Some(parsed) = table.deserialize::<TerraformTable>() else {
            return Self::default();
        };
        Self {
            include: parsed.include.map(|include| table.patterns(&include, Glob::new)),
            ignore_unresolved: table.patterns(&parsed.ignore_unresolved, NamePattern::new),
        }
    }
}

impl LanguageOptions for TerraformConfig {
    fn include(&self) -> Option<&[Glob]> {
        self.include.as_deref()
    }

    fn backend(&self, settings: BackendSettings<'_>) -> Box<dyn Backend> {
        Box::new(TerraformBackend::new(settings.root, self.ignore_unresolved.clone()))
    }
}

/// Terraform, OpenTofu and Terragrunt.
// The table as written, which is its schema.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, expecting = "a table")]
struct TerraformTable {
    /// Globs of the files to scan. Defaults to every Terraform, OpenTofu and
    /// Terragrunt file: `**/*.tf`, `**/*.tofu` and `**/terragrunt.hcl`.
    include: Option<Vec<Located<String>>>,

    /// Name patterns of module sources and dependency paths that are expected
    /// not to resolve, so are not reported as unresolved.
    ignore_unresolved: Vec<Located<String>>,
}
