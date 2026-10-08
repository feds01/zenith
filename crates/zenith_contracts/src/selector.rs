//! Selectors, and the units they resolve to.
//!
//! | Form | Matches |
//! |---|---|
//! | `web/src/domains/*` | internal directories and files (zenith's glob dialect) |
//! | `pkg:@acme/*` | the directories of matching workspace packages |
//! | `npm:@mastra/*` | external packages |
//! | `node:*`, `bun:*` | builtins |
//!
//! A selector resolves to a list of [Unit]s: each matched directory is one
//! unit containing its whole subtree, and every other matched node is a unit
//! of one. When a glob matches nested nodes, only the outermost matches are
//! kept, since they already contain the inner ones.

use zenith_config::SelectorConfig;
use zenith_graph::{Graph, ModuleId, NodeKind, NodeSet};
use zenith_index::Index;
use zenith_utils::glob::{Glob, NamePattern, escape};

/// A labelled set of nodes that a contract reasons about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// How the unit is shown in output: a path, a package name, or an
    /// external/builtin id.
    pub label: String,
    /// The node the unit was matched from, if any.
    pub root: Option<ModuleId>,
    /// The nodes that can carry imports: files, externals and builtins.
    pub members: NodeSet,
}

#[derive(Debug, Clone)]
enum SelectorKind {
    Path(Glob),
    Package(NamePattern),
    External(NamePattern),
    Builtin { scheme: &'static str, pattern: NamePattern },
}

/// A compiled selector.
#[derive(Debug, Clone)]
pub struct Selector {
    raw: String,
    kind: SelectorKind,
    except: Vec<Selector>,
    directive: Option<String>,
}

impl Selector {
    /// Parse a string selector.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        let error = |message: String| format!("invalid selector `{raw}`: {message}");
        let kind = if let Some(name) = raw.strip_prefix("pkg:") {
            SelectorKind::Package(NamePattern::new(name).map_err(error)?)
        } else if let Some(name) = raw.strip_prefix("npm:") {
            SelectorKind::External(NamePattern::new(name).map_err(error)?)
        } else if let Some(name) = raw.strip_prefix("node:") {
            SelectorKind::Builtin {
                scheme: "node",
                pattern: NamePattern::new(name).map_err(error)?,
            }
        } else if let Some(name) = raw.strip_prefix("bun:") {
            SelectorKind::Builtin { scheme: "bun", pattern: NamePattern::new(name).map_err(error)? }
        } else {
            if raw.starts_with('/') || raw.split('/').any(|segment| segment == "..") {
                return Err(error("paths must be relative to the project root".to_string()));
            }
            SelectorKind::Path(Glob::new(raw.trim_end_matches('/')).map_err(error)?)
        };
        Ok(Self { raw: raw.to_string(), kind, except: Vec::new(), directive: None })
    }

    /// Compile a selector from configuration.
    pub fn from_config(config: &SelectorConfig) -> Result<Self, String> {
        match config {
            SelectorConfig::Pattern(raw) => Self::parse(raw),
            SelectorConfig::Table(table) => {
                let mut selector = Self::parse(&table.path)?;
                selector.except =
                    table.except.iter().map(|raw| Self::parse(raw)).collect::<Result<_, _>>()?;
                selector.directive = table.directive.clone();
                if selector.directive.is_some()
                    && !matches!(selector.kind, SelectorKind::Path(_) | SelectorKind::Package(_))
                {
                    return Err(format!(
                        "selector `{}`: `directive` only applies to files",
                        table.path
                    ));
                }
                Ok(selector)
            }
        }
    }

    /// Parse a path selector relative to a container directory.
    pub fn relative_to(container: &str, raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if raw.contains(':') {
            return Err(format!(
                "`{raw}`: layers of a contract with `containers` must be paths relative to each container"
            ));
        }
        let mut selector = Self::parse(&format!("{}/{raw}", escape(container)))?;
        selector.raw = format!("{container}/{raw}");
        Ok(selector)
    }

    /// The selector as written.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Whether the selector must match something. `npm:` and `node:`
    /// selectors may legitimately match nothing: it just means nothing
    /// imports that package.
    pub fn must_match(&self) -> bool {
        matches!(self.kind, SelectorKind::Path(_) | SelectorKind::Package(_))
    }

    /// Resolve the selector against the project.
    pub fn resolve(&self, index: &Index) -> Vec<Unit> {
        self.resolve_with_base(index).0
    }

    /// Resolve the selector, also reporting whether the base selector (before
    /// `except` and `directive` filtering) matched anything. A selector whose
    /// base matched is never "unmatched", even if filtering left nothing.
    pub fn resolve_with_base(&self, index: &Index) -> (Vec<Unit>, bool) {
        let graph = &index.graph;
        let mut units: Vec<Unit> = match &self.kind {
            SelectorKind::Path(glob) => resolve_path(graph, glob)
                .into_iter()
                .map(|id| Unit {
                    label: graph.name(id).to_string(),
                    root: Some(id),
                    members: graph.members(id),
                })
                .collect(),
            SelectorKind::Package(pattern) => {
                let root_package = index
                    .workspace
                    .root_package
                    .as_ref()
                    .filter(|root| pattern.is_literal() && pattern.is_match(&root.name));
                index
                    .workspace
                    .packages
                    .iter()
                    .filter(|package| pattern.is_match(&package.name))
                    .chain(root_package)
                    .filter_map(|package| {
                        let id = if package.dir.is_empty() {
                            Some(ModuleId::ROOT)
                        } else {
                            graph.id(&package.dir)
                        }?;
                        Some(Unit {
                            label: package.name.clone(),
                            root: Some(id),
                            members: graph.members(id),
                        })
                    })
                    .collect()
            }
            SelectorKind::External(pattern) => leaf_units(graph, NodeKind::External, |name| {
                name.strip_prefix("npm:").is_some_and(|name| pattern.is_match(name))
            }),
            SelectorKind::Builtin { scheme, pattern } => {
                leaf_units(graph, NodeKind::Builtin, |name| {
                    name.split_once(':')
                        .is_some_and(|(s, name)| s == *scheme && pattern.is_match(name))
                })
            }
        };

        let base_matched = !units.is_empty();

        if !self.except.is_empty() {
            let mut excluded = NodeSet::new();
            for selector in &self.except {
                for unit in selector.resolve(index) {
                    excluded.union_with(&unit.members);
                }
            }
            for unit in &mut units {
                unit.members.difference_with(&excluded);
            }
        }

        if let Some(directive) = &self.directive {
            units = units
                .iter()
                .flat_map(|unit| unit.members.iter())
                .filter(|id| graph.node(*id).directives.iter().any(|d| d == directive))
                .map(|id| Unit {
                    label: graph.name(id).to_string(),
                    root: Some(id),
                    members: NodeSet::single(id),
                })
                .collect();
        }

        (units, base_matched)
    }
}

/// The outermost internal nodes matched by a glob.
fn resolve_path(graph: &Graph, glob: &Glob) -> Vec<ModuleId> {
    // A fully literal glob may name a file without its extension, so search
    // from its parent directory.
    let mut prefix = glob.literal_prefix();
    if glob.is_literal() {
        prefix = prefix.rsplit_once('/').map_or(String::new(), |(parent, _)| parent.to_string());
    }
    let start = if prefix.is_empty() {
        ModuleId::ROOT
    } else {
        match graph.id(&prefix) {
            Some(id) => id,
            None => return Vec::new(),
        }
    };

    let subtree = graph.subtree(start);
    let mut matched = Vec::new();
    let mut index = 0;
    while index < subtree.len() {
        let id = subtree[index];
        let node = graph.node(id);
        if id != ModuleId::ROOT && node.is_internal() && node_matches(glob, &node.name, node.kind) {
            matched.push(id);
            index += graph.subtree(id).len();
            continue;
        }
        index += 1;
    }
    matched
}

/// Whether a glob matches a node: its path, or for a JS/TS file, its path
/// without the extension.
pub(crate) fn node_matches(glob: &Glob, name: &str, kind: NodeKind) -> bool {
    glob.is_match(name)
        || (kind == NodeKind::File
            && strip_source_extension(name).is_some_and(|s| glob.is_match(s)))
}

/// Remove a JS/TS extension from a path, e.g. `a/b.d.ts` is `a/b`.
pub(crate) fn strip_source_extension(name: &str) -> Option<&str> {
    if let Some(stripped) = name.strip_suffix(".d.ts") {
        return Some(stripped);
    }
    let (stem, extension) = name.rsplit_once('.')?;
    (zenith_parse::SOURCE_EXTENSIONS.contains(&extension)
        && !stem.ends_with('/')
        && !stem.is_empty())
    .then_some(stem)
}

fn leaf_units(graph: &Graph, kind: NodeKind, matches: impl Fn(&str) -> bool) -> Vec<Unit> {
    graph
        .ids()
        .filter(|id| graph.node(*id).kind == kind && matches(graph.name(*id)))
        .map(|id| Unit {
            label: graph.name(id).to_string(),
            root: Some(id),
            members: NodeSet::single(id),
        })
        .collect()
}

/// Union the members of several units.
pub(crate) fn union_members<'u>(units: impl IntoIterator<Item = &'u Unit>) -> NodeSet {
    let mut members = NodeSet::new();
    for unit in units {
        members.union_with(&unit.members);
    }
    members
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_source_extensions() {
        assert_eq!(strip_source_extension("a/types.ts"), Some("a/types"));
        assert_eq!(strip_source_extension("a/types.d.ts"), Some("a/types"));
        assert_eq!(strip_source_extension("a/Button.tsx"), Some("a/Button"));
        assert_eq!(strip_source_extension("a/styles.css"), None);
        assert_eq!(strip_source_extension("a/.eslintrc.js"), Some("a/.eslintrc"));
    }

    #[test]
    fn parses_selector_forms() {
        assert!(Selector::parse("web/src/*").unwrap().must_match());
        assert!(Selector::parse("pkg:@acme/*").unwrap().must_match());
        assert!(!Selector::parse("npm:sharp").unwrap().must_match());
        assert!(!Selector::parse("node:*").unwrap().must_match());
        assert!(Selector::parse("/abs").is_err());
        assert!(Selector::parse("../up").is_err());
        assert!(Selector::relative_to("web/src", "pkg:x").is_err());
        assert_eq!(
            Selector::relative_to("app/(group)", "lib").unwrap().as_str(),
            "app/(group)/lib"
        );
    }
}
