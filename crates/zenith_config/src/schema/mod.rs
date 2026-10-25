//! The `zenith.toml` file as written, which is its schema.
//!
//! These types mirror the file one-to-one: serde deserialises them, rejecting
//! unknown keys everywhere, and schemars describes them in the JSON Schema of
//! the file, so their docs are written for the people editing it. Values that
//! are checked after deserialisation, such as selectors, are [`Located`], so
//! that errors can point at them.

use std::{borrow::Cow, fmt};

use indexmap::IndexMap;
use schemars::{
    JsonSchema, Schema, SchemaGenerator, generate::SchemaSettings, transform::RecursiveTransform,
};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, IgnoredAny, MapAccess, Visitor, value::MapAccessDeserializer},
};
use serde_json::{Map, Value, json};
use zenith_graph::ImportKind;
use zenith_utils::location::ByteRange;

use crate::{
    config::{DEFAULT_BASELINE_PATH, DEFAULT_CACHE_DIR, StaleLevel},
    contracts::{AlertLevel, Reexports, Severity},
    language::Languages,
};

/// A value with the byte range that it was written at.
#[derive(Debug, Default)]
pub struct Located<T> {
    /// The value.
    pub value: T,

    /// Its byte range in the file. A default, which was not written, has an
    /// empty range.
    pub range: ByteRange,
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Located<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let spanned = toml::Spanned::<T>::deserialize(deserializer)?;
        Ok(Self { range: spanned.span().into(), value: spanned.into_inner() })
    }
}

/// Serialised as the bare value, which is how schemars shows defaults.
impl<T: Serialize> Serialize for Located<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(serializer)
    }
}

/// Described as the bare value, since the location is not written.
impl<T: JsonSchema> JsonSchema for Located<T> {
    fn inline_schema() -> bool {
        T::inline_schema()
    }

    fn schema_name() -> Cow<'static, str> {
        T::schema_name()
    }

    fn schema_id() -> Cow<'static, str> {
        T::schema_id()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        T::json_schema(generator)
    }
}

/// The configuration of a zenith project. It lives at the project root, which
/// all of its paths are relative to.
// Each table is deserialised on its own, so that every table reports its
// errors, and each contract's errors name it and point into it. This struct
// only rejects unknown tables, and describes the tables for the schema.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, expecting = "a table")]
#[schemars(title = "zenith.toml")]
#[expect(dead_code, reason = "the tables are deserialised on their own")]
pub(crate) struct ConfigFile {
    /// Which files to scan for imports, whatever their language.
    #[serde(default)]
    #[schemars(with = "FilesTable")]
    pub(crate) files: IgnoredAny,

    /// The languages to scan, each in a table of its own. A language is
    /// enabled when its table is written, even if empty. Without a
    /// `[languages]` table, the default languages are enabled with their
    /// defaults; an empty one enables nothing, which is an error.
    #[serde(default)]
    #[schemars(with = "LanguagesTable")]
    pub(crate) languages: IgnoredAny,

    /// Defaults that apply to every contract.
    #[serde(default)]
    #[schemars(with = "AnalysisTable")]
    pub(crate) analysis: IgnoredAny,

    /// The baseline of known violations, which `zenith check` ignores so that
    /// only new violations fail it.
    #[serde(default)]
    #[schemars(with = "BaselineTable")]
    pub(crate) baseline: IgnoredAny,

    /// The cache of what zenith found in each file on earlier runs, which
    /// lets it skip reading and parsing the files that haven't changed since.
    #[serde(default)]
    #[schemars(with = "CacheTable")]
    pub(crate) cache: IgnoredAny,

    /// The architecture contracts, keyed by id. Ids may contain ASCII
    /// letters, digits, `-` and `_`, and are what `--contract`, the baseline
    /// and `zenith-ignore` comments refer to.
    #[serde(default)]
    #[schemars(with = "IndexMap<String, ContractTable>")]
    pub(crate) contracts: IgnoredAny,
}

/// Which files to scan for imports, whatever their language.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, expecting = "a table")]
pub(crate) struct FilesTable {
    /// Globs of files and directories to skip, on top of the default
    /// excludes, such as `node_modules` and `.next`.
    pub(crate) exclude: Vec<Located<String>>,

    /// Whether to skip the files that `.gitignore`, `.ignore` and the global
    /// git excludes ignore.
    pub(crate) respect_gitignore: bool,
}

impl Default for FilesTable {
    fn default() -> Self {
        Self { exclude: Vec::new(), respect_gitignore: true }
    }
}

/// The languages to scan, each in a table of its own.
// Each language parses its own table, so that the errors in it keep their
// location, and so that giving a language twice, under two of its names, can
// be reported. The tables are those of the languages that zenith is given,
// which [`json_schema`] adds to the schema of this struct.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, expecting = "a table")]
pub(crate) struct LanguagesTable {}

/// Defaults that apply to every contract.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, expecting = "a table")]
pub(crate) struct AnalysisTable {
    /// The import kinds that contracts ignore unless they set their own
    /// `exclude_kinds`, e.g. `["type"]` to exempt type-only imports.
    pub(crate) exclude_kinds: Vec<ImportKind>,

    /// How contracts see the imports of pass-through modules, the files that
    /// only re-export other modules, unless they set their own `reexports`.
    pub(crate) reexports: Reexports,
}

/// The baseline of known violations.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, expecting = "a table")]
pub(crate) struct BaselineTable {
    /// The path of the baseline file.
    pub(crate) path: String,

    /// How to report baseline entries whose import no longer exists:
    /// `"error"` fails the check, so that the baseline can only shrink, and
    /// `"warn"` reports a warning.
    pub(crate) stale: StaleLevel,
}

impl Default for BaselineTable {
    fn default() -> Self {
        Self { path: DEFAULT_BASELINE_PATH.to_string(), stale: StaleLevel::default() }
    }
}

/// The cache of what zenith found in each file on earlier runs.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, expecting = "a table")]
pub(crate) struct CacheTable {
    /// Whether to use the cache. `--no-cache` turns it off for one run.
    pub(crate) enabled: bool,

    /// The directory to keep the cache in. Zenith creates it, with a
    /// `.gitignore` that ignores everything in it.
    pub(crate) dir: String,
}

impl Default for CacheTable {
    fn default() -> Self {
        Self { enabled: true, dir: DEFAULT_CACHE_DIR.to_string() }
    }
}

/// An architecture contract. Its `type` says what it checks.
#[derive(Debug, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ContractTable {
    /// The `from` modules must not import the `to` modules, directly or
    /// through other modules.
    Forbidden(ForbiddenTable),

    /// The modules must not depend on one another, directly or through other
    /// modules.
    Independence(IndependenceTable),

    /// Higher layers may import lower layers, never the reverse.
    Layers(LayersTable),

    /// Only the allowed importers, and the modules themselves, may directly
    /// import the modules.
    Protected(ProtectedTable),

    /// The modules must not form dependency cycles.
    Acyclic(AcyclicTable),

    /// Code outside each module may only import it through its entry points.
    Encapsulation(EncapsulationTable),

    /// Each module may only be imported by so many other modules, and import
    /// so many.
    Limits(LimitsTable),
}

impl ContractTable {
    /// The options that every contract type shares.
    pub(crate) fn shared(&self) -> SharedOptions<'_> {
        match self {
            Self::Forbidden(table) => table.shared(),
            Self::Independence(table) => table.shared(),
            Self::Layers(table) => table.shared(),
            Self::Protected(table) => table.shared(),
            Self::Acyclic(table) => table.shared(),
            Self::Encapsulation(table) => table.shared(),
            Self::Limits(table) => table.shared(),
        }
    }
}

/// The options that every contract type shares, as written.
pub(crate) struct SharedOptions<'a> {
    /// The title for output.
    pub(crate) name: Option<&'a str>,

    /// How severe it is when the contract is broken.
    pub(crate) severity: Severity,

    /// The import kinds to ignore, if set.
    pub(crate) exclude_kinds: Option<&'a [ImportKind]>,

    /// How to see imports through re-exports, if set.
    pub(crate) reexports: Option<&'a Located<Reexports>>,

    /// The imports to ignore.
    pub(crate) ignore_imports: &'a [Located<String>],

    /// How to report an `ignore_imports` entry that matches no import.
    pub(crate) unmatched_ignore_imports: AlertLevel,

    /// Text shown under the violations.
    pub(crate) guidance: Option<&'a str>,
}

/// Declares the table of a contract type: its own keys, followed by the
/// options that every contract type shares. The shared options are spelled
/// out in each table, rather than flattened into it, because serde can only
/// reject unknown keys, and keep their locations, without flattening.
macro_rules! contract_table {
    (
        $(#[$meta:meta])*
        $name:ident {
            $(
                $(#[$field_meta:meta])*
                $field:ident: $ty:ty,
            )*
        }
    ) => {
        #[derive(Debug, Deserialize, JsonSchema)]
        $(#[$meta])*
        #[serde(deny_unknown_fields, expecting = "a contract table")]
        pub(crate) struct $name {
            $(
                $(#[$field_meta])*
                pub(crate) $field: $ty,
            )*

            /// The title shown in output. Defaults to the contract id.
            pub(crate) name: Option<String>,

            /// `"warn"` reports the contract without failing the check when it
            /// is broken, for gradual rollout.
            #[serde(default)]
            pub(crate) severity: Severity,

            /// Import kinds to ignore, e.g. `["type"]` to exempt type-only
            /// imports. Defaults to `[analysis] exclude_kinds`.
            pub(crate) exclude_kinds: Option<Vec<ImportKind>>,

            /// How to see the imports of pass-through modules, the files that only
            /// re-export other modules: `"opaque"`, an import of a pass-through module
            /// imports every module it re-exports, or `"transparent"`, an
            /// import of names from it imports the modules that define them.
            /// Defaults to `[analysis] reexports`.
            #[serde(default)]
            #[schemars(with = "Reexports")]
            pub(crate) reexports: Option<Located<Reexports>>,

            /// Imports to ignore, as `"<selector> -> <selector>"`: the direct
            /// imports from the first selector's modules to the second's are
            /// removed before checking.
            #[serde(default)]
            pub(crate) ignore_imports: Vec<Located<String>>,

            /// How to report an `ignore_imports` entry that matches no import:
            /// `"error"` breaks the contract, `"warn"` reports a warning, and
            /// `"none"` says nothing.
            #[serde(default)]
            pub(crate) unmatched_ignore_imports: AlertLevel,

            /// Text shown under the violations, e.g. how to fix them.
            pub(crate) guidance: Option<String>,
        }

        impl $name {
            /// The options that every contract type shares.
            fn shared(&self) -> SharedOptions<'_> {
                SharedOptions {
                    name: self.name.as_deref(),
                    severity: self.severity,
                    exclude_kinds: self.exclude_kinds.as_deref(),
                    reexports: self.reexports.as_ref(),
                    ignore_imports: &self.ignore_imports,
                    unmatched_ignore_imports: self.unmatched_ignore_imports,
                    guidance: self.guidance.as_deref(),
                }
            }
        }
    };
}

contract_table! {
    /// A `forbidden` contract.
    ForbiddenTable {
        /// The modules that must not import the `to` modules.
        from: Vec<Located<SelectorValue>>,

        /// The modules that must not be imported. External packages (`npm:`)
        /// and builtins (`node:`, `bun:`) are allowed, with `js` enabled.
        to: Vec<Located<SelectorValue>>,

        /// If `true`, only direct imports count, rather than chains of
        /// imports too.
        #[serde(default)]
        allow_indirect_imports: bool,

        /// Modules that chains of imports may start or end at, but not pass
        /// through, e.g. `{ path = "web/**", directive = "use server" }`:
        /// client code that imports a server function gets a reference to
        /// it, not the code that it imports. Has no effect on direct imports,
        /// so it can't be set with `allow_indirect_imports`.
        #[serde(default)]
        boundaries: Located<Vec<Located<SelectorValue>>>,
    }
}

contract_table! {
    /// An `independence` contract.
    IndependenceTable {
        /// The modules that must not depend on one another.
        modules: Vec<Located<SelectorValue>>,

        /// If `true`, only direct imports count, rather than chains of
        /// imports too.
        #[serde(default)]
        allow_indirect_imports: bool,

        /// Modules that chains of imports may start or end at, but not pass
        /// through, e.g. `{ path = "web/**", directive = "use server" }`:
        /// client code that imports a server function gets a reference to
        /// it, not the code that it imports. Has no effect on direct imports,
        /// so it can't be set with `allow_indirect_imports`.
        #[serde(default)]
        boundaries: Located<Vec<Located<SelectorValue>>>,
    }
}

contract_table! {
    /// A `layers` contract.
    LayersTable {
        /// The layers, from highest to lowest. Higher layers may import lower
        /// layers, never the reverse.
        layers: Located<Vec<Located<LayerValue>>>,

        /// If set, the layers are paths relative to each container, and each
        /// container is checked on its own.
        #[serde(default)]
        containers: Vec<Located<SelectorValue>>,

        /// Whether every child of every container must be declared as a
        /// layer. Requires `containers`.
        #[serde(default)]
        exhaustive: Located<bool>,

        /// Children exempt from `exhaustive`: name patterns matched against
        /// the base name with or without its JS/TS extension, so `index`
        /// matches `index.ts`. The index files of the enabled languages, such
        /// as Rust's `lib.rs`, `main.rs` and `mod.rs`, are always exempt.
        #[serde(default)]
        exhaustive_ignores: Vec<Located<String>>,

        /// If `true`, only direct imports count, rather than chains of
        /// imports too.
        #[serde(default)]
        allow_indirect_imports: bool,

        /// Modules that chains of imports may start or end at, but not pass
        /// through, e.g. `{ path = "web/**", directive = "use server" }`:
        /// client code that imports a server function gets a reference to
        /// it, not the code that it imports. Has no effect on direct imports,
        /// so it can't be set with `allow_indirect_imports`.
        #[serde(default)]
        boundaries: Located<Vec<Located<SelectorValue>>>,
    }
}

contract_table! {
    /// A `protected` contract.
    ProtectedTable {
        /// The protected modules.
        modules: Vec<Located<SelectorValue>>,

        /// The only modules that may directly import the protected modules,
        /// besides the protected modules themselves.
        allowed_importers: Vec<Located<SelectorValue>>,
    }
}

contract_table! {
    /// An `acyclic` contract.
    AcyclicTable {
        /// The modules that must not form dependency cycles, e.g. `["pkg:*"]`.
        modules: Vec<Located<SelectorValue>>,

        /// How many further generations of children to check: with `1`, the
        /// children of each directory module must not form cycles either.
        #[serde(default)]
        depth: u32,

        /// Modules whose children are not checked.
        #[serde(default)]
        skip: Vec<Located<SelectorValue>>,
    }
}

contract_table! {
    /// An `encapsulation` contract.
    EncapsulationTable {
        /// The encapsulated modules, which must be directories.
        modules: Vec<Located<SelectorValue>>,

        /// Globs, relative to each module, of the files that code outside it
        /// may import. They ignore JS/TS extensions, so `index` matches
        /// `index.ts`, and match files only: `public/**` exposes a directory.
        #[serde(default = "default_entrypoints")]
        entrypoints: Vec<Located<String>>,

        /// Modules that may import any file of the encapsulated modules, e.g.
        /// tests.
        #[serde(default)]
        allowed_importers: Vec<Located<SelectorValue>>,
    }
}

contract_table! {
    /// A `limits` contract, with `max_fan_in`, `max_fan_out` or both.
    #[schemars(extend("anyOf" = [{ "required": ["max_fan_in"] }, { "required": ["max_fan_out"] }]))]
    LimitsTable {
        /// The modules whose imports are counted, each on its own.
        modules: Vec<Located<SelectorValue>>,

        /// The most files outside each module that may import it. A file
        /// counts once, however many of the module's files it imports.
        max_fan_in: Option<usize>,

        /// The most files and packages of the project outside each module
        /// that it may import. External packages and builtins don't count. A
        /// file or package counts once, however many of the module's files
        /// import it.
        max_fan_out: Option<usize>,
    }
}

/// The default entry points of an `encapsulation` contract.
fn default_entrypoints() -> Vec<Located<String>> {
    vec![Located { value: "index".to_string(), range: ByteRange::default() }]
}

/// A selector of modules: a path glob such as `web/src/domains/*`, a
/// workspace package (`pkg:@acme/*`), an external package (`npm:react`), a
/// builtin (`node:*` or `bun:*`), or a table that filters one. External
/// packages and builtins belong to the `js` language, which must be enabled.
#[derive(Debug, JsonSchema)]
#[serde(untagged)]
pub(crate) enum SelectorValue {
    /// A selector.
    String(String),

    /// A selector with filters.
    Table(SelectorTable),
}

/// A selector with filters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, expecting = "a selector table")]
pub(crate) struct SelectorTable {
    /// The selector, in any of its string forms.
    pub(crate) path: Located<String>,

    /// Selectors of modules to remove from each matched module.
    #[serde(default)]
    pub(crate) except: Vec<Located<String>>,

    /// Keep only the files that start with this directive, such as
    /// `"use client"`, each as a module of its own.
    pub(crate) directive: Option<Located<String>>,
}

impl<'de> Deserialize<'de> for SelectorValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(StringOrTable {
            expecting: "a selector string or table",
            string: Self::String,
            table: Self::Table,
        })
    }
}

/// A layer of a `layers` contract: `"a"`, `"a | b"` for independent siblings
/// that may not import each other, `"a : b"` for siblings that may, and
/// `"(a)"` for a member that may be missing. Members are selectors.
#[derive(Debug, JsonSchema)]
#[serde(untagged)]
pub(crate) enum LayerValue {
    /// A layer.
    String(String),

    /// A layer with options.
    Table(LayerTable),
}

/// A layer with options.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, expecting = "a layer table")]
pub(crate) struct LayerTable {
    /// The layer, in its string form.
    pub(crate) layer: Located<String>,

    /// Whether the layer is closed: the layers above it may not import the
    /// layers below it.
    #[serde(default)]
    pub(crate) closed: bool,
}

impl<'de> Deserialize<'de> for LayerValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(StringOrTable {
            expecting: "a layer string or table",
            string: Self::String,
            table: Self::Table,
        })
    }
}

/// Deserialises a string, or a table as `T`, into a `V`. Unlike an untagged
/// enum, this deserialises the table directly, so that its errors name the
/// offending key and keep their location.
struct StringOrTable<T, V> {
    /// What the value should be, for errors.
    expecting: &'static str,

    /// Makes the value of the string form.
    string: fn(String) -> V,

    /// Makes the value of the table form.
    table: fn(T) -> V,
}

impl<'de, T: Deserialize<'de>, V> Visitor<'de> for StringOrTable<T, V> {
    type Value = V;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.expecting)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<V, E> {
        Ok((self.string)(value.to_string()))
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<V, A::Error> {
        T::deserialize(MapAccessDeserializer::new(map)).map(self.table)
    }
}

/// The JSON Schema of the configuration file, which editors use to complete
/// and check it, with a table for each of the `languages`.
pub fn json_schema(languages: Languages) -> Value {
    let mut generator = SchemaSettings::draft2020_12()
        .with_transform(RecursiveTransform(remove_null))
        .into_generator();

    // 1. Describe the table of each language under each of its names, which adds
    //    its definition to those of the schema.
    let mut tables = Map::new();
    for language in languages.iter() {
        let mut table = (language.schema)(&mut generator).to_value();
        for (index, name) in language.table_names.iter().enumerate() {
            table["description"] = match index {
                0 => json!(language.description),
                _ => {
                    json!(format!("Another name for `{}`. Giving both is an error.", language.name))
                }
            };
            tables.insert(name.to_string(), table.clone());
        }
    }

    // 2. Describe the file, and give `[languages]` the tables.
    let mut schema = generator.into_root_schema_for::<ConfigFile>().to_value();
    schema["$defs"]["LanguagesTable"]["properties"] = Value::Object(tables);
    schema
}

/// Remove `null` from a schema. schemars allows it in every `Option`, with no
/// setting to stop it, but TOML has no null: an optional key is left out
/// instead.
fn remove_null(schema: &mut Schema) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    if object.get("default").is_some_and(Value::is_null) {
        object.remove("default");
    }
    if let Some(Value::Array(types)) = object.get_mut("type") {
        types.retain(|kind| kind != "null");
        if let [kind] = types.as_slice() {
            let kind = kind.clone();
            object.insert("type".to_string(), kind);
        }
    }
}

#[cfg(test)]
mod tests;
