//! JavaScript and TypeScript in the configuration: the [`JS`] language, the
//! `[languages.js]` table, and the options it is parsed into.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::Deserialize;
use zenith_config::{Language, LanguageOptions, LanguageTable, Located};
use zenith_utils::glob::{Glob, NamePattern};

/// JavaScript and TypeScript.
pub const JS: Language = Language {
    name: "js",
    table_names: &["js", "typescript"],
    description: "JavaScript and TypeScript: how their files are found and their imports resolved.",
    enabled_by_default: true,
    include: &["**/*.{ts,tsx,mts,cts,js,jsx,mjs,cjs}"],
    // Declaration files (`.d.ts`) are `ts` files.
    source_extensions: &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"],
    optional_extensions: &["ts", "tsx", "mts", "cts", "d.ts", "js", "jsx", "mjs", "cjs"],
    schemes: &["npm", "node", "bun"],
    parse: |table| Arc::new(JsConfig::parse(table)),
    defaults: || Arc::new(JsConfig::default()),
    schema: |generator| generator.subschema_for::<JsTable>(),
};

/// The options of JavaScript and TypeScript. `None` options leave the
/// defaults in place.
#[derive(Debug, Clone, Default)]
pub struct JsConfig {
    /// Globs of the files to scan. `None` scans every file with one of the
    /// [`JS`] source extensions.
    pub include: Option<Vec<Glob>>,

    /// Which `tsconfig.json` files to use.
    pub tsconfig: Tsconfig,

    /// The condition names to match in `exports` maps, replacing the
    /// defaults.
    pub conditions: Option<Vec<String>>,

    /// The `package.json` fields that point at a package's entry point,
    /// replacing the defaults.
    pub main_fields: Option<Vec<String>>,

    /// The extensions to try when a specifier has none, in order, replacing
    /// the defaults.
    pub extensions: Option<Vec<String>>,

    /// Bundler-only aliases, in the order of the file: a specifier prefix,
    /// and the path relative to the project root that it maps to.
    pub alias: IndexMap<String, String>,

    /// Patterns of specifiers that are expected not to resolve, so are not
    /// reported as unresolved.
    pub ignore_unresolved: Vec<NamePattern>,
}

/// Which `tsconfig.json` files the resolver uses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Tsconfig {
    /// Each file uses its nearest `tsconfig.json`.
    #[default]
    Auto,

    /// No `tsconfig.json` is used.
    Off,

    /// Every file uses this `tsconfig.json`, a path relative to the project
    /// root.
    File(String),
}

impl JsConfig {
    /// Parse the table of JavaScript and TypeScript. An invalid table has
    /// the defaults, so that the language is still enabled.
    fn parse(table: &mut LanguageTable<'_, '_>) -> Self {
        let Some(parsed) = table.deserialize::<JsTable>() else {
            return Self::default();
        };
        let tsconfig = match parsed.tsconfig.as_deref() {
            None | Some("auto") => Tsconfig::Auto,
            Some("off") => Tsconfig::Off,
            Some(path) => Tsconfig::File(path.to_string()),
        };
        Self {
            include: parsed.include.map(|include| table.patterns(&include, Glob::new)),
            tsconfig,
            conditions: parsed.conditions,
            main_fields: parsed.main_fields,
            extensions: parsed.extensions,
            alias: parsed.alias,
            ignore_unresolved: table.patterns(&parsed.ignore_unresolved, NamePattern::new),
        }
    }
}

impl LanguageOptions for JsConfig {
    fn include(&self) -> Option<&[Glob]> {
        self.include.as_deref()
    }
}

/// JavaScript and TypeScript.
// The table as written, which is its schema.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, expecting = "a table")]
struct JsTable {
    /// Globs of the files to scan. Defaults to every JavaScript and
    /// TypeScript file, `**/*.{ts,tsx,mts,cts,js,jsx,mjs,cjs}`.
    include: Option<Vec<Located<String>>>,

    /// Which `tsconfig.json` files to use: `"auto"` (the default) uses the
    /// nearest one of each file, `"off"` uses none, and any other value is
    /// the path of the one to use for every file.
    tsconfig: Option<String>,

    /// The condition names to match in `exports` maps, replacing the
    /// defaults: `types`, `source`, `import`, `module`, `require`, `node`
    /// and `default`. Conditions are matched in the order that each package
    /// lists them.
    conditions: Option<Vec<String>>,

    /// The `package.json` fields that point at a package's entry point,
    /// replacing the defaults: `module`, then `main`.
    main_fields: Option<Vec<String>>,

    /// The extensions to try, in order, when a specifier has none, replacing
    /// the defaults: `.ts`, `.tsx`, `.d.ts`, `.mts`, `.cts`, `.js`, `.jsx`,
    /// `.mjs`, `.cjs` and `.json`.
    extensions: Option<Vec<String>>,

    /// Bundler-only aliases: a specifier prefix, and the path that it maps
    /// to.
    alias: IndexMap<String, String>,

    /// Name patterns of specifiers that are expected not to resolve, so are
    /// not reported as unresolved, e.g. `"/*"` for files served from a public
    /// directory, or `"virtual:*"`.
    ignore_unresolved: Vec<Located<String>>,
}
