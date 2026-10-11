//! Parsing `zenith.toml` and validating it into a [`Config`].
//!
//! The file is parsed into a TOML document that keeps the location of every
//! key and value. Each table is deserialised from it on its own, and so is
//! each contract: its `type` picks the table to deserialise the rest of it
//! into, so that its errors point at the offending key. Deserialising a table
//! stops at its first error, but the other tables still report theirs. Every
//! value is then validated, and the errors are collected in the order of the
//! file.

use std::ops::Range;

use serde::Deserialize;
use toml::{
    Spanned,
    de::{DeTable, DeValue, Deserializer, ValueDeserializer},
};
use zenith_utils::glob::{Glob, GlobError, NamePattern};

use crate::{
    config::{
        AcyclicConfig, AnalysisConfig, BaselineConfig, Config, ContractConfig, ContractKind,
        EncapsulationConfig, FilesConfig, ForbiddenConfig, IndependenceConfig, LayersConfig,
        ProtectedConfig, ResolveConfig, Tsconfig,
    },
    error::{ConfigError, ConfigErrorKind, Location, classify, suggest},
    file::{
        AcyclicTable, AnalysisTable, BaselineTable, CONTRACT_TYPES, ConfigFile, ContractTable,
        EncapsulationTable, FilesTable, ForbiddenTable, IndependenceTable, LayerValue, LayersTable,
        Located, ProtectedTable, ResolveTable, SelectorValue,
    },
    layer::{Layer, LayerError},
    selector::{IgnoreImport, Selector, SelectorTarget},
};

/// Parse and validate the contents of a configuration file. On failure,
/// returns the errors, in the order of the file: the first error in the
/// structure of each table and contract, such as an unknown key, and every
/// invalid value, such as a selector.
pub fn parse(contents: &str) -> Result<Config, Vec<ConfigError>> {
    let mut parser = Parser { contents, errors: Vec::new(), contract: None };
    match parser.config() {
        Some(config) if parser.errors.is_empty() => Ok(config),
        _ => {
            let mut errors = parser.errors;
            errors.sort_by_key(|error| error.location.map(|location| location.range.start()));
            Err(errors)
        }
    }
}

/// Whether a contract id is valid.
fn is_valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Validates a configuration file, collecting its errors.
struct Parser<'a> {
    /// The contents of the file.
    contents: &'a str,

    /// The errors found so far.
    errors: Vec<ConfigError>,

    /// The id of the contract being validated, which errors are attributed
    /// to.
    contract: Option<String>,
}

impl Parser<'_> {
    /// Record an error at a byte range of the file.
    fn error(&mut self, kind: ConfigErrorKind, span: Option<&Range<usize>>) {
        self.errors.push(ConfigError {
            kind,
            contract: self.contract.clone(),
            location: span.map(|span| Location::new(self.contents, span)),
        });
    }

    /// The value of `result`, or else record its error at `span`.
    fn check<T, E>(
        &mut self,
        result: Result<T, E>,
        span: Option<&Range<usize>>,
        kind: impl FnOnce(E) -> ConfigErrorKind,
    ) -> Option<T> {
        result.map_err(|error| self.error(kind(error), span)).ok()
    }

    /// The location of a byte range of the file.
    fn locate(&self, span: &Range<usize>) -> Location {
        Location::new(self.contents, span)
    }

    /// Record an error of the TOML deserialiser.
    fn deserialise_error(&mut self, error: &toml::de::Error) {
        self.error(classify(error.message()), error.span().as_ref());
    }

    /// Parse the whole file. Returns `None` if it isn't valid TOML.
    fn config(&mut self) -> Option<Config> {
        let mut document = match DeTable::parse(self.contents) {
            Ok(document) => document,
            Err(error) => {
                self.error(
                    ConfigErrorKind::Syntax { message: error.message().to_string() },
                    error.span().as_ref(),
                );
                return None;
            }
        };

        let tables = document.get_mut();
        let files = tables.remove("files");
        let resolve = tables.remove("resolve");
        let analysis = tables.remove("analysis");
        let baseline = tables.remove("baseline");
        let contracts = tables.remove("contracts");
        // Only unknown tables are left.
        if let Err(error) = ConfigFile::deserialize(Deserializer::from(document)) {
            self.deserialise_error(&error);
        }

        let mut config = Config::default();
        if let Some(table) = self.table(files) {
            config.files = self.files(table);
        }
        if let Some(table) = self.table(resolve) {
            config.resolve = self.resolve(table);
        }
        if let Some(table) = self.table::<AnalysisTable>(analysis) {
            config.analysis =
                AnalysisConfig { exclude_kinds: table.exclude_kinds.into_iter().collect() };
        }
        if let Some(table) = self.table::<BaselineTable>(baseline) {
            config.baseline = BaselineConfig { path: table.path, stale: table.stale };
        }
        if let Some(contracts) = contracts {
            config.contracts = self.contracts(contracts, &config.analysis);
        }
        Some(config)
    }

    /// Deserialise a global table, or its defaults when it isn't written.
    /// Returns `None` if it is invalid.
    fn table<'de, T: Deserialize<'de> + Default>(
        &mut self,
        value: Option<Spanned<DeValue<'de>>>,
    ) -> Option<T> {
        let Some(value) = value else {
            return Some(T::default());
        };
        T::deserialize(ValueDeserializer::from(value))
            .inspect_err(|error| self.deserialise_error(error))
            .ok()
    }

    /// Validate `[files]`.
    fn files(&mut self, table: FilesTable) -> FilesConfig {
        FilesConfig {
            include: table.include.map(|include| self.patterns(&include, Glob::new)),
            exclude: self.patterns(&table.exclude, Glob::new),
            respect_gitignore: table.respect_gitignore,
        }
    }

    /// Validate `[resolve]`.
    fn resolve(&mut self, table: ResolveTable) -> ResolveConfig {
        let tsconfig = match table.tsconfig.as_deref() {
            None | Some("auto") => Tsconfig::Auto,
            Some("off") => Tsconfig::Off,
            Some(path) => Tsconfig::File(path.to_string()),
        };
        ResolveConfig {
            tsconfig,
            conditions: table.conditions,
            main_fields: table.main_fields,
            extensions: table.extensions,
            alias: table.alias,
            ignore_unresolved: self.patterns(&table.ignore_unresolved, NamePattern::new),
        }
    }

    /// Deserialise and validate every contract.
    fn contracts(
        &mut self,
        contracts: Spanned<DeValue<'_>>,
        analysis: &AnalysisConfig,
    ) -> Vec<ContractConfig> {
        let span = contracts.span();
        let DeValue::Table(contracts) = contracts.into_inner() else {
            self.error(ConfigErrorKind::NotATable { key: "contracts".to_string() }, Some(&span));
            return Vec::new();
        };

        let mut validated = Vec::new();
        for (id, value) in contracts {
            let id_span = id.span();
            let id = id.into_inner().into_owned();
            if !is_valid_id(&id) {
                self.error(ConfigErrorKind::InvalidContractId { id: id.clone() }, Some(&id_span));
            }

            let span = value.span();
            let DeValue::Table(table) = value.into_inner() else {
                self.error(
                    ConfigErrorKind::NotATable { key: format!("contracts.{id}") },
                    Some(&span),
                );
                continue;
            };

            self.contract = Some(id.clone());
            if let Some(table) = self.contract_table(table, span) {
                validated.push(self.contract(id, &table, analysis));
            }
            self.contract = None;
        }
        validated
    }

    /// Deserialise a contract, at `span`, into the table of its `type`.
    fn contract_table(
        &mut self,
        mut table: DeTable<'_>,
        span: Range<usize>,
    ) -> Option<ContractTable> {
        let Some(kind) = table.remove("type") else {
            self.error(ConfigErrorKind::MissingKey { key: "type".to_string() }, Some(&span));
            return None;
        };
        let kind_span = kind.span();
        let DeValue::String(kind) = kind.into_inner() else {
            self.error(ConfigErrorKind::ContractTypeNotAString, Some(&kind_span));
            return None;
        };

        let value = ValueDeserializer::from(Spanned::new(span, DeValue::Table(table)));
        let table = match kind.as_ref() {
            "forbidden" => ForbiddenTable::deserialize(value).map(ContractTable::Forbidden),
            "independence" => {
                IndependenceTable::deserialize(value).map(ContractTable::Independence)
            }
            "layers" => LayersTable::deserialize(value).map(ContractTable::Layers),
            "protected" => ProtectedTable::deserialize(value).map(ContractTable::Protected),
            "acyclic" => AcyclicTable::deserialize(value).map(ContractTable::Acyclic),
            "encapsulation" => {
                EncapsulationTable::deserialize(value).map(ContractTable::Encapsulation)
            }
            name => {
                let suggestion = suggest(name, CONTRACT_TYPES).map(String::from);
                self.error(
                    ConfigErrorKind::UnknownContractType {
                        name: name.to_string(),
                        expected: CONTRACT_TYPES.map(String::from).to_vec(),
                        suggestion,
                    },
                    Some(&kind_span),
                );
                return None;
            }
        };
        table.inspect_err(|error| self.deserialise_error(error)).ok()
    }

    /// Validate a contract.
    fn contract(
        &mut self,
        id: String,
        table: &ContractTable,
        analysis: &AnalysisConfig,
    ) -> ContractConfig {
        let shared = table.shared();
        let exclude_kinds = shared
            .exclude_kinds
            .map_or(analysis.exclude_kinds, |kinds| kinds.iter().copied().collect());
        let ignore_imports = self.ignore_imports(shared.ignore_imports);

        let kind = match table {
            ContractTable::Forbidden(table) => ContractKind::Forbidden(ForbiddenConfig {
                from: self.selectors(&table.from),
                to: self.selectors(&table.to),
                allow_indirect_imports: table.allow_indirect_imports,
            }),
            ContractTable::Independence(table) => ContractKind::Independence(IndependenceConfig {
                modules: self.selectors(&table.modules),
                allow_indirect_imports: table.allow_indirect_imports,
            }),
            ContractTable::Layers(table) => ContractKind::Layers(self.layers(table)),
            ContractTable::Protected(table) => ContractKind::Protected(ProtectedConfig {
                modules: self.selectors(&table.modules),
                allowed_importers: self.selectors(&table.allowed_importers),
            }),
            ContractTable::Acyclic(table) => ContractKind::Acyclic(AcyclicConfig {
                modules: self.selectors(&table.modules),
                depth: table.depth,
                skip: self.selectors(&table.skip),
            }),
            ContractTable::Encapsulation(table) => {
                ContractKind::Encapsulation(EncapsulationConfig {
                    modules: self.selectors(&table.modules),
                    entrypoints: self.patterns(&table.entrypoints, Glob::new),
                    allowed_importers: self.selectors(&table.allowed_importers),
                })
            }
        };

        ContractConfig {
            name: shared.name.unwrap_or(&id).to_string(),
            id,
            severity: shared.severity,
            exclude_kinds,
            ignore_imports,
            unmatched_ignore_imports: shared.unmatched_ignore_imports,
            guidance: shared.guidance.map(String::from),
            kind,
        }
    }

    /// Validate the options of a `layers` contract.
    fn layers(&mut self, table: &LayersTable) -> LayersConfig {
        if table.layers.value.is_empty() {
            self.error(ConfigErrorKind::NoLayers, Some(&table.layers.span));
        }
        let relative = !table.containers.is_empty();
        if table.exhaustive.value && !relative {
            self.error(ConfigErrorKind::ExhaustiveWithoutContainers, Some(&table.exhaustive.span));
        }
        LayersConfig {
            layers: table
                .layers
                .value
                .iter()
                .filter_map(|layer| self.layer(layer, relative))
                .collect(),
            containers: self.selectors(&table.containers),
            exhaustive: table.exhaustive.value,
            exhaustive_ignores: self.patterns(&table.exhaustive_ignores, NamePattern::new),
            allow_indirect_imports: table.allow_indirect_imports,
        }
    }

    /// Validate a layer. With `relative`, its members must be paths, relative
    /// to each container.
    fn layer(&mut self, value: &Located<LayerValue>, relative: bool) -> Option<Layer> {
        let (raw, span, closed) = match &value.value {
            LayerValue::String(raw) => (raw, &value.span, false),
            LayerValue::Table(table) => (&table.layer.value, &table.layer.span, table.closed),
        };
        let layer = Layer::parse(raw, closed).and_then(|layer| {
            let not_relative = layer.members.iter().find(|member| {
                relative && !matches!(member.selector.target(), SelectorTarget::Path(_))
            });
            match not_relative {
                Some(member) => {
                    Err(LayerError::NotRelative { member: member.selector.as_str().to_string() })
                }
                None => Ok(layer.at(self.locate(span))),
            }
        });
        self.check(layer, Some(span), |error| ConfigErrorKind::InvalidLayer {
            layer: raw.clone(),
            error,
        })
    }

    /// Validate a list of selectors, leaving out the invalid ones.
    fn selectors(&mut self, values: &[Located<SelectorValue>]) -> Vec<Selector> {
        values.iter().filter_map(|value| self.selector(value)).collect()
    }

    /// Validate a selector.
    fn selector(&mut self, value: &Located<SelectorValue>) -> Option<Selector> {
        let table = match &value.value {
            SelectorValue::String(raw) => return self.string_selector(raw, &value.span),
            SelectorValue::Table(table) => table,
        };
        let selector = self.string_selector(&table.path.value, &table.path.span);
        let except = table
            .except
            .iter()
            .filter_map(|except| self.string_selector(&except.value, &except.span))
            .collect();
        let directive = table.directive.as_ref();
        let selector =
            selector?.with_filters(except, directive.map(|directive| directive.value.clone()));
        self.check(selector, directive.map(|directive| &directive.span), |error| {
            ConfigErrorKind::InvalidSelector { selector: table.path.value.clone(), error }
        })
    }

    /// Validate the string form of a selector.
    fn string_selector(&mut self, raw: &str, span: &Range<usize>) -> Option<Selector> {
        let selector = Selector::parse(raw).map(|selector| selector.at(self.locate(span)));
        self.check(selector, Some(span), |error| ConfigErrorKind::InvalidSelector {
            selector: raw.to_string(),
            error,
        })
    }

    /// Validate `ignore_imports` entries, leaving out the invalid ones.
    fn ignore_imports(&mut self, entries: &[Located<String>]) -> Vec<IgnoreImport> {
        entries
            .iter()
            .filter_map(|entry| {
                let ignore = IgnoreImport::parse(&entry.value)
                    .map(|ignore| ignore.at(self.locate(&entry.span)));
                self.check(ignore, Some(&entry.span), |error| {
                    ConfigErrorKind::InvalidIgnoreImport { entry: entry.value.clone(), error }
                })
            })
            .collect()
    }

    /// Compile globs or name patterns, leaving out the invalid ones.
    fn patterns<T>(
        &mut self,
        patterns: &[Located<String>],
        compile: impl Fn(&str) -> Result<T, GlobError>,
    ) -> Vec<T> {
        patterns
            .iter()
            .filter_map(|pattern| {
                self.check(compile(&pattern.value), Some(&pattern.span), |error| {
                    ConfigErrorKind::InvalidPattern { pattern: pattern.value.clone(), error }
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_contract_type_is_parsed() {
        for name in CONTRACT_TYPES {
            let options = match name {
                "forbidden" => "from = []\nto = []",
                "layers" => "layers = [\"a\"]",
                "protected" => "modules = []\nallowed_importers = []",
                _ => "modules = []",
            };
            let contents = format!("[contracts.c]\ntype = \"{name}\"\n{options}\n");
            match parse(&contents) {
                Ok(config) => assert_eq!(config.contracts[0].kind.type_name(), name),
                Err(errors) => panic!("`{name}` does not parse: {errors:?}"),
            }
        }
    }
}
