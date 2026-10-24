//! Parsing the contracts. Each contract's `type` picks the table that the rest
//! of it is deserialised into, and its options are then validated.

use serde::Deserialize;
use toml::{
    Spanned,
    de::{DeTable, DeValue, ValueDeserializer},
};
use zenith_utils::{
    glob::{Glob, NamePattern},
    location::ByteRange,
};

use super::{Parser, TomlValue};
use crate::{
    config::AnalysisConfig,
    contracts::{
        AcyclicConfig, CONTRACT_TYPES, ContractConfig, ContractKind, EncapsulationConfig,
        ForbiddenConfig, IndependenceConfig, LayersConfig, LimitsConfig, ProtectedConfig,
        Reexports,
    },
    error::{Alternatives, ConfigErrorKind},
    schema::{
        AcyclicTable, ContractTable, EncapsulationTable, ForbiddenTable, IndependenceTable,
        LayersTable, LimitsTable, Located, ProtectedTable, SelectorValue,
    },
    selector::Selector,
};

impl Parser {
    /// Parse `[contracts]`, leaving out the contracts whose table is invalid.
    pub(super) fn parse_contracts(
        &mut self,
        contracts: TomlValue<'_>,
        analysis: &AnalysisConfig,
    ) -> Vec<ContractConfig> {
        let range = ByteRange::from(contracts.span());
        let DeValue::Table(contracts) = contracts.into_inner() else {
            let kind = ConfigErrorKind::NotATable { key: "contracts".to_string() };
            self.add_error(kind, Some(range));
            return Vec::new();
        };

        let mut parsed = Vec::new();
        for (id, value) in contracts {
            let id_range = ByteRange::from(id.span());
            let id = id.into_inner().into_owned();
            if !ContractConfig::is_valid_id(&id) {
                let kind = ConfigErrorKind::InvalidContractId { id: id.clone() };
                self.add_error(kind, Some(id_range));
            }

            let span = value.span();
            let DeValue::Table(table) = value.into_inner() else {
                let kind = ConfigErrorKind::NotATable { key: format!("contracts.{id}") };
                self.add_error(kind, Some(span.into()));
                continue;
            };

            let contract = self.in_contract(&id, |parser| {
                let table = parser.parse_contract_table(Spanned::new(span, table))?;
                Some(parser.parse_contract(id.clone(), id_range, &table, analysis))
            });
            parsed.extend(contract);
        }
        parsed
    }

    /// Deserialise a contract into the table of its `type`.
    fn parse_contract_table(&mut self, table: Spanned<DeTable<'_>>) -> Option<ContractTable> {
        let span = table.span();
        let mut table = table.into_inner();
        let Some(kind) = table.remove("type") else {
            let kind = ConfigErrorKind::MissingKey { key: "type".to_string() };
            self.add_error(kind, Some(span.into()));
            return None;
        };
        let kind_range = ByteRange::from(kind.span());
        let DeValue::String(kind) = kind.into_inner() else {
            self.add_error(ConfigErrorKind::ContractTypeNotAString, Some(kind_range));
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
            "limits" => LimitsTable::deserialize(value).map(ContractTable::Limits),
            name => {
                let expected = CONTRACT_TYPES.map(String::from).to_vec();
                let kind = ConfigErrorKind::UnknownContractType {
                    name: name.to_string(),
                    alternatives: Alternatives::new(name, expected),
                };
                self.add_error(kind, Some(kind_range));
                return None;
            }
        };
        table.inspect_err(|error| self.add_toml_error(error)).ok()
    }

    /// Validate the options of the contract `id`, whose key is at `range`,
    /// with the defaults of `analysis`.
    fn parse_contract(
        &mut self,
        id: String,
        range: ByteRange,
        table: &ContractTable,
        analysis: &AnalysisConfig,
    ) -> ContractConfig {
        let shared = table.shared();
        let exclude_kinds = shared
            .exclude_kinds
            .map_or(analysis.exclude_kinds, |kinds| kinds.iter().copied().collect());
        let ignore_imports = self.parse_ignore_imports(shared.ignore_imports);
        let mut reexports =
            shared.reexports.map_or(analysis.reexports, |reexports| reexports.value);

        let kind = match table {
            ContractTable::Forbidden(table) => ContractKind::Forbidden(ForbiddenConfig {
                from: self.parse_selectors(&table.from),
                to: self.parse_selectors(&table.to),
                allow_indirect_imports: table.allow_indirect_imports,
                boundaries: self.parse_boundaries(&table.boundaries, table.allow_indirect_imports),
            }),
            ContractTable::Independence(table) => ContractKind::Independence(IndependenceConfig {
                modules: self.parse_selectors(&table.modules),
                allow_indirect_imports: table.allow_indirect_imports,
                boundaries: self.parse_boundaries(&table.boundaries, table.allow_indirect_imports),
            }),
            ContractTable::Layers(table) => ContractKind::Layers(self.parse_layers(table)),
            ContractTable::Protected(table) => ContractKind::Protected(ProtectedConfig {
                modules: self.parse_selectors(&table.modules),
                allowed_importers: self.parse_selectors(&table.allowed_importers),
            }),
            ContractTable::Acyclic(table) => ContractKind::Acyclic(AcyclicConfig {
                modules: self.parse_selectors(&table.modules),
                depth: table.depth,
                skip: self.parse_selectors(&table.skip),
            }),
            ContractTable::Encapsulation(table) => {
                // Importing through an entry point, which often only re-exports, is what
                // it allows, so it never sees through them.
                if let Some(Located { value: Reexports::Transparent, range }) = shared.reexports {
                    self.add_error(ConfigErrorKind::TransparentEncapsulation, Some(*range));
                }
                reexports = Reexports::Opaque;
                ContractKind::Encapsulation(EncapsulationConfig {
                    modules: self.parse_selectors(&table.modules),
                    entrypoints: self.parse_patterns(&table.entrypoints, Glob::new),
                    allowed_importers: self.parse_selectors(&table.allowed_importers),
                })
            }
            ContractTable::Limits(table) => {
                if table.max_fan_in.is_none() && table.max_fan_out.is_none() {
                    self.add_error(ConfigErrorKind::NoLimits, Some(range));
                }
                ContractKind::Limits(LimitsConfig {
                    modules: self.parse_selectors(&table.modules),
                    max_fan_in: table.max_fan_in,
                    max_fan_out: table.max_fan_out,
                })
            }
        };

        ContractConfig {
            name: shared.name.unwrap_or(&id).to_string(),
            id,
            range,
            severity: shared.severity,
            exclude_kinds,
            reexports,
            ignore_imports,
            unmatched_ignore_imports: shared.unmatched_ignore_imports,
            guidance: shared.guidance.map(String::from),
            kind,
        }
    }

    /// Validate the options of a `layers` contract.
    fn parse_layers(&mut self, table: &LayersTable) -> LayersConfig {
        if table.layers.value.is_empty() {
            self.add_error(ConfigErrorKind::NoLayers, Some(table.layers.range));
        }
        let relative = !table.containers.is_empty();
        if table.exhaustive.value && !relative {
            self.add_error(
                ConfigErrorKind::ExhaustiveWithoutContainers,
                Some(table.exhaustive.range),
            );
        }
        LayersConfig {
            layers: table
                .layers
                .value
                .iter()
                .filter_map(|layer| self.parse_layer(layer, relative))
                .collect(),
            containers: self.parse_selectors(&table.containers),
            exhaustive: table.exhaustive.value,
            exhaustive_ignores: self.parse_patterns(&table.exhaustive_ignores, NamePattern::new),
            allow_indirect_imports: table.allow_indirect_imports,
            boundaries: self.parse_boundaries(&table.boundaries, table.allow_indirect_imports),
        }
    }

    /// Parse the `boundaries` of a contract that follows chains, unless
    /// `allow_indirect_imports` turns the chains off.
    fn parse_boundaries(
        &mut self,
        boundaries: &Located<Vec<Located<SelectorValue>>>,
        allow_indirect_imports: bool,
    ) -> Vec<Selector> {
        if allow_indirect_imports && !boundaries.value.is_empty() {
            self.add_error(ConfigErrorKind::BoundariesWithoutChains, Some(boundaries.range));
        }
        self.parse_selectors(&boundaries.value)
    }
}
