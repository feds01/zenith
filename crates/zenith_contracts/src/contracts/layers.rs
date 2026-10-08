//! `layers`: higher layers may import lower layers, never the reverse.

use zenith_config::{LayerConfig, LayersConfig};
use zenith_graph::{
    GraphView, KindSet, Level, ModuleId, NodeKind, PackageDependency, Route,
    find_illegal_dependencies_for_layers, find_illegal_direct_dependencies_for_layers,
};
use zenith_index::Index;
use zenith_utils::glob::NamePattern;

use super::resolve_units;
use crate::{
    findings::{Chain, Link, Violation},
    selector::{Selector, Unit, strip_source_extension, union_members},
};

#[derive(Debug, Clone)]
pub(crate) struct Layers {
    /// One architecture per container (or a single one without containers).
    containers: Vec<Architecture>,
    allow_indirect: bool,
}

#[derive(Debug, Clone)]
struct Architecture {
    levels: Vec<Level>,
    /// The label of each layer, by level.
    labels: Vec<Vec<String>>,
    /// The container's children that no layer declares (exhaustive
    /// contracts only).
    undeclared: Vec<(String, String)>,
}

/// A parsed layer string.
struct LayerSpec {
    items: Vec<(String, bool)>,
    independent: bool,
    closed: bool,
}

fn parse_layer(config: &LayerConfig) -> Result<LayerSpec, String> {
    let (raw, closed) = match config {
        LayerConfig::Layer(raw) => (raw.as_str(), false),
        LayerConfig::Table { layer, closed } => (layer.as_str(), *closed),
    };

    // `:` only separates siblings when surrounded by whitespace, since it
    // also appears in `pkg:` and `npm:` selectors.
    let has_pipe = raw.contains('|');
    let colon_parts: Vec<&str> = split_spaced_colon(raw);
    let has_colon = colon_parts.len() > 1;
    if has_pipe && has_colon {
        return Err(format!("layer `{raw}` mixes `|` and `:` separators"));
    }
    let parts: Vec<&str> = if has_pipe { raw.split('|').collect() } else { colon_parts };

    let mut items = Vec::new();
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            return Err(format!("layer `{raw}` has an empty entry"));
        }
        match part.strip_prefix('(').and_then(|p| p.strip_suffix(')')) {
            Some(inner) => items.push((inner.trim().to_string(), true)),
            None => items.push((part.to_string(), false)),
        }
    }
    Ok(LayerSpec { items, independent: !has_colon, closed })
}

fn split_spaced_colon(raw: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut last = 0;
    let bytes = raw.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b':'
            && index > 0
            && bytes[index - 1].is_ascii_whitespace()
            && bytes.get(index + 1).is_some_and(|b| b.is_ascii_whitespace())
        {
            parts.push(&raw[last..index]);
            last = index + 1;
        }
    }
    parts.push(&raw[last..]);
    parts
}

impl Layers {
    pub(crate) fn compile(index: &Index, config: &LayersConfig) -> Result<Self, String> {
        let specs = config.layers.iter().map(parse_layer).collect::<Result<Vec<_>, _>>()?;
        if specs.is_empty() {
            return Err("`layers` cannot be empty".to_string());
        }
        if config.exhaustive && config.containers.is_empty() {
            return Err("`exhaustive` requires `containers`".to_string());
        }
        let ignores = config
            .exhaustive_ignores
            .iter()
            .map(|raw| NamePattern::new(raw))
            .collect::<Result<Vec<_>, _>>()?;

        let mut containers = Vec::new();
        if config.containers.is_empty() {
            containers.push(Self::compile_architecture(
                index,
                None,
                &specs,
                config.exhaustive,
                &ignores,
            )?);
        } else {
            for container in resolve_units(index, "containers", &config.containers)? {
                let root = container
                    .root
                    .filter(|root| index.graph.node(*root).kind == NodeKind::Directory);
                let Some(root) = root else {
                    return Err(format!("container `{}` is not a directory", container.label));
                };
                containers.push(Self::compile_architecture(
                    index,
                    Some(root),
                    &specs,
                    config.exhaustive,
                    &ignores,
                )?);
            }
        }

        Ok(Self { containers, allow_indirect: config.allow_indirect_imports })
    }

    fn compile_architecture(
        index: &Index,
        container: Option<ModuleId>,
        specs: &[LayerSpec],
        exhaustive: bool,
        exhaustive_ignores: &[NamePattern],
    ) -> Result<Architecture, String> {
        let graph = &index.graph;
        let container_name = container.map(|id| graph.name(id).to_string());

        let mut levels = Vec::new();
        let mut labels = Vec::new();
        let mut roots: Vec<ModuleId> = Vec::new();
        let mut all: Vec<Unit> = Vec::new();

        for spec in specs {
            let mut layers = Vec::new();
            let mut level_labels = Vec::new();
            for (item, optional) in &spec.items {
                let selector = match &container_name {
                    Some(container) => Selector::relative_to(container, item)?,
                    None => Selector::parse(item)?,
                };
                let units = selector.resolve(index);
                if units.is_empty() && selector.must_match() && !optional {
                    return Err(match &container_name {
                        Some(container) => {
                            format!(
                                "missing layer `{item}` in container `{container}`; wrap it in parentheses to make it optional"
                            )
                        }
                        None => format!("layer `{item}` does not match anything"),
                    });
                }
                roots.extend(units.iter().filter_map(|unit| unit.root));
                let members = union_members(&units);
                let label = selector.as_str().to_string();
                all.push(Unit { label: label.clone(), root: None, members: members.clone() });
                layers.push(members);
                level_labels.push(label);
            }
            levels.push(Level { layers, independent: spec.independent, closed: spec.closed });
            labels.push(level_labels);
        }

        for (index, a) in all.iter().enumerate() {
            for b in &all[index + 1..] {
                if a.members.intersects(&b.members) {
                    return Err(format!("layers `{}` and `{}` overlap", a.label, b.label));
                }
            }
        }

        let mut undeclared = Vec::new();
        if exhaustive && let (Some(container), Some(container_name)) = (container, &container_name)
        {
            for child in graph.children(container) {
                let declared = roots.iter().any(|root| {
                    root == child
                        || graph.is_in_subtree(*root, *child)
                        || graph.is_in_subtree(*child, *root)
                });
                let node = graph.node(*child);
                let base = node.base_name();
                let stem = strip_source_extension(base).unwrap_or(base);
                let ignored =
                    exhaustive_ignores.iter().any(|p| p.is_match(base) || p.is_match(stem));
                if !declared && !ignored {
                    undeclared.push((container_name.clone(), node.name.clone()));
                }
            }
        }

        Ok(Architecture { levels, labels, undeclared })
    }

    pub(crate) fn check(&self, view: &GraphView, kinds: KindSet) -> Vec<Violation> {
        let mut violations = Vec::new();
        for architecture in &self.containers {
            let dependencies = if self.allow_indirect {
                find_illegal_direct_dependencies_for_layers(view, &architecture.levels)
            } else {
                find_illegal_dependencies_for_layers(view, &architecture.levels)
            };
            for PackageDependency { importer, imported, routes } in dependencies {
                violations.push(Violation::Dependency {
                    importer: architecture.labels[importer.level][importer.layer].clone(),
                    imported: architecture.labels[imported.level][imported.layer].clone(),
                    chains: routes.iter().map(|route| route_to_chain(view, kinds, route)).collect(),
                });
            }
            for (container, module) in &architecture.undeclared {
                violations.push(Violation::UndeclaredModule {
                    container: container.clone(),
                    module: module.clone(),
                });
            }
        }
        violations
    }
}

/// Convert a grimp route into a chain in import-linter's notation.
pub(crate) fn route_to_chain(view: &GraphView, kinds: KindSet, route: &Route) -> Chain {
    let mut heads = route.heads.clone();
    let mut tails = route.tails.clone();
    heads.sort();
    tails.sort();

    if route.middle.is_empty() {
        return Chain {
            links: vec![Link::new(view, kinds, heads[0], tails[0])],
            extra_heads: Vec::new(),
            extra_tails: Vec::new(),
        };
    }

    let first = route.middle[0];
    let last = *route.middle.last().unwrap();
    let mut path = vec![heads[0]];
    path.extend(&route.middle);
    path.push(tails[0]);
    let mut chain = Chain::from_path(view, kinds, &path);
    chain.extra_heads =
        heads[1..].iter().map(|head| Link::new(view, kinds, *head, first)).collect();
    chain.extra_tails = tails[1..].iter().map(|tail| Link::new(view, kinds, last, *tail)).collect();
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(raw: &str) -> LayerSpec {
        parse_layer(&LayerConfig::Layer(raw.to_string())).unwrap()
    }

    #[test]
    fn parses_layer_strings() {
        let s = spec("a | b");
        assert_eq!(s.items, vec![("a".to_string(), false), ("b".to_string(), false)]);
        assert!(s.independent);

        let s = spec("pkg:@acme/types : pkg:@acme/data");
        assert_eq!(s.items.len(), 2);
        assert!(!s.independent);
        assert_eq!(s.items[0].0, "pkg:@acme/types");

        let s = spec("(optional)");
        assert_eq!(s.items, vec![("optional".to_string(), true)]);

        let s = spec("pkg:web");
        assert_eq!(s.items, vec![("pkg:web".to_string(), false)]);

        assert!(parse_layer(&LayerConfig::Layer("a | b : c".into())).is_err());
        assert!(parse_layer(&LayerConfig::Layer("a | ".into())).is_err());
    }
}
