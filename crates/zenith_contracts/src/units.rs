//! The units that selectors resolve to.
//!
//! A [`Selector`] resolves into a list of [`Unit`]s:
//!
//! - each matched directory is one unit, with its whole subtree;
//! - each matched file, external package or builtin is a unit of one;
//! - `except` removes members from each unit;
//! - `directive` replaces the units with one unit per member file that has the
//!   directive.
//!
//! When a path glob matches nested nodes, such as `web/**` matching both
//! `web/src` and `web/src/app.ts`, only the outermost become units, since they
//! contain the inner ones.

use zenith_config::{SOURCE_EXTENSIONS, Selector, SelectorTarget};
use zenith_graph::{Graph, ModuleId, Node, NodeKind, NodeSet};
use zenith_utils::{
    glob::{Glob, NamePattern},
    path::posix_parent,
};
use zenith_workspace::Workspace;

/// A labelled set of the nodes that take part in imports, which contracts
/// reason about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// How the unit is shown: the path of a directory or file, the name of a
    /// workspace package, or the name of an external package or builtin, e.g.
    /// `npm:react`.
    pub label: String,

    /// The node the unit was matched from: a directory, a file, an external
    /// package or a builtin.
    pub root: ModuleId,

    /// The files in the subtree of a directory, or the node itself.
    pub members: NodeSet,
}

impl Unit {
    /// The unit of a node, labelled by its name.
    fn of_node(graph: &Graph, id: ModuleId) -> Self {
        Self { label: graph.name(id).to_string(), root: id, members: graph.members(id) }
    }
}

/// The units that a selector resolves to in a project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// The units: in tree order for a path selector, by package directory for
    /// a `pkg:` selector, and by name otherwise, or for a selector with a
    /// directive.
    pub units: Vec<Unit>,

    /// Whether the selector's path or name matched anything, before `except`
    /// and `directive` filtered the units. A selector that must match
    /// something is unmatched only when this is false: filters that leave
    /// nothing don't make it a typo.
    pub matched: bool,
}

impl Selection {
    /// Resolve `selector` against a project's `graph` and the `workspace`
    /// packages that `pkg:` selectors match.
    pub fn resolve(selector: &Selector, graph: &Graph, workspace: &Workspace) -> Self {
        let mut units = match selector.target() {
            SelectorTarget::Path(glob) => {
                match_paths(graph, glob).into_iter().map(|id| Unit::of_node(graph, id)).collect()
            }
            SelectorTarget::Package(pattern) => package_units(graph, workspace, pattern),
            SelectorTarget::External(pattern) => {
                leaf_units(graph, NodeKind::External, "npm", pattern)
            }
            SelectorTarget::Builtin { scheme, pattern } => {
                leaf_units(graph, NodeKind::Builtin, scheme.as_str(), pattern)
            }
        };
        let matched = !units.is_empty();

        if !selector.except().is_empty() {
            let mut excluded = NodeSet::new();
            for except in selector.except() {
                excluded.union_with(&Self::resolve(except, graph, workspace).members());
            }
            for unit in &mut units {
                unit.members.difference_with(&excluded);
            }
        }

        if let Some(directive) = selector.directive() {
            // Units can overlap, e.g. nested packages, so collect each file once.
            let mut files = NodeSet::new();
            for unit in &units {
                files.extend(unit.members.iter().filter(|id| {
                    graph.node(*id).directives().iter().any(|found| found == directive)
                }));
            }
            units = files.iter().map(|id| Unit::of_node(graph, id)).collect();
        }

        Self { units, matched }
    }

    /// Every member of every unit.
    pub fn members(&self) -> NodeSet {
        let mut members = NodeSet::new();
        for unit in &self.units {
            members.union_with(&unit.members);
        }
        members
    }
}

/// The outermost internal nodes that `glob` matches, in tree order.
fn match_paths(graph: &Graph, glob: &Glob) -> Vec<ModuleId> {
    let prefix = glob.literal_prefix();

    // A literal glob names one path, which may be a file's without its
    // extension, so only the children of its parent can match.
    if glob.is_literal() {
        let Some(parent) = graph.id(posix_parent(&prefix)) else {
            return Vec::new();
        };
        return graph
            .children(parent)
            .iter()
            .copied()
            .filter(|id| matches(glob, graph.node(*id)))
            .collect();
    }

    // Otherwise, every match is in the subtree of the literal prefix. Once a node
    // matches, its descendants are skipped.
    let Some(start) = graph.id(&prefix) else {
        return Vec::new();
    };
    let subtree = graph.subtree(start);
    let mut matched = Vec::new();
    let mut position = 0;
    while let Some(&id) = subtree.get(position) {
        if matches(glob, graph.node(id)) {
            matched.push(id);
            position += graph.subtree(id).len();
        } else {
            position += 1;
        }
    }
    matched
}

/// Whether `glob` matches an internal node: its path, or for a file, its
/// path without its JavaScript or TypeScript extension, so `src/types`
/// matches `src/types.ts`.
fn matches(glob: &Glob, node: &Node) -> bool {
    match node.kind() {
        NodeKind::Directory => glob.is_match(node.name()),
        NodeKind::File => {
            glob.is_match(node.name())
                || strip_source_extension(node.name()).is_some_and(|stem| glob.is_match(stem))
        }
        NodeKind::External | NodeKind::Builtin => false,
    }
}

/// A file's path without its JavaScript or TypeScript extension, e.g.
/// `src/types` for `src/types.d.ts`. `None` for other files, and for files
/// whose name is only an extension, e.g. `src/.ts`.
fn strip_source_extension(path: &str) -> Option<&str> {
    let stem = match path.strip_suffix(".d.ts") {
        Some(stem) => stem,
        None => {
            let (stem, extension) = path.rsplit_once('.')?;
            SOURCE_EXTENSIONS.contains(&extension).then_some(stem)?
        }
    };
    (!stem.is_empty() && !stem.ends_with('/')).then_some(stem)
}

/// The units of the workspace packages whose name matches `pattern`, in order
/// of their directories. `pkg:*` never matches the root package, which would
/// contain everything, so only its exact name does, and it comes last.
fn package_units(graph: &Graph, workspace: &Workspace, pattern: &NamePattern) -> Vec<Unit> {
    let root = workspace
        .root_package
        .as_ref()
        .filter(|root| pattern.is_literal() && pattern.is_match(&root.name));
    workspace
        .packages
        .iter()
        .filter(|package| pattern.is_match(&package.name))
        .chain(root)
        .filter_map(|package| {
            // A package with no files in the graph has no node.
            let id = graph.id(&package.dir)?;
            Some(Unit { label: package.name.clone(), ..Unit::of_node(graph, id) })
        })
        .collect()
}

/// The units of the external packages or builtins of `scheme`, e.g. `npm`,
/// whose name matches `pattern`, by name.
fn leaf_units(graph: &Graph, kind: NodeKind, scheme: &str, pattern: &NamePattern) -> Vec<Unit> {
    graph
        .ids()
        .filter(|id| {
            let node = graph.node(*id);
            node.kind() == kind
                && node
                    .name()
                    .split_once(':')
                    .is_some_and(|(prefix, name)| prefix == scheme && pattern.is_match(name))
        })
        .map(|id| Unit::of_node(graph, id))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use thin_vec::thin_vec;
    use zenith_config::{Config, ContractKind};
    use zenith_graph::GraphBuilder;
    use zenith_workspace::WorkspacePackage;

    use super::*;

    /// The graph and workspace packages of a project.
    struct Project {
        /// The module tree and import graph.
        graph: Graph,

        /// The workspace packages.
        workspace: Workspace,
    }

    impl Project {
        /// Resolve `selector` in the project.
        fn select(&self, selector: &Selector) -> Selection {
            Selection::resolve(selector, &self.graph, &self.workspace)
        }
    }

    /// A project with web sources, nested workspace packages, external
    /// packages and builtins.
    fn project() -> Project {
        let mut builder = GraphBuilder::new();
        for file in [
            "web/src/app/layout.tsx",
            "web/src/env.d.ts",
            "web/src/lib/format.test.ts",
            "web/src/lib/format.ts",
            "web/src/types.ts",
            "web/src/types/user.ts",
            "packages/api/project.ts",
            "packages/ui/project.ts",
            "packages/ui/README.md",
        ] {
            builder.add_file(file).unwrap();
        }
        for file in ["web/src/app/page.tsx", "packages/ui/button.tsx", "packages/ui/icons/star.tsx"]
        {
            let file = builder.add_file(file).unwrap();
            builder.set_directives(file, thin_vec!["use client".to_string()]);
        }
        for name in ["npm:react", "npm:@acme/legacy"] {
            builder.add_external(name).unwrap();
        }
        for name in ["node:fs", "bun:sqlite"] {
            builder.add_builtin(name).unwrap();
        }

        let package = |name: &str, dir: &str| WorkspacePackage {
            name: name.to_string(),
            dir: dir.to_string(),
            path: PathBuf::from("/repo").join(dir),
        };
        let workspace = Workspace {
            // Sorted by directory, as discovered. `@acme/empty` has no files.
            packages: vec![
                package("@acme/api", "packages/api"),
                package("@acme/empty", "packages/empty"),
                package("@acme/ui", "packages/ui"),
                package("@acme/icons", "packages/ui/icons"),
            ],
            root_package: Some(package("acme", "")),
        };

        Project { graph: builder.finish(), workspace }
    }

    /// A selector in its string form.
    fn selector(raw: &str) -> Selector {
        Selector::parse(raw).unwrap()
    }

    /// A selector in its table form, e.g. `{ path = "web", except = [...] }`.
    fn table(toml: &str) -> Selector {
        let config = Config::parse(&format!(
            "[contracts.a]\ntype = \"forbidden\"\nfrom = [{toml}]\nto = [\"npm:x\"]\n"
        ))
        .unwrap();
        let ContractKind::Forbidden(forbidden) = &config.contracts[0].kind else {
            panic!("expected a forbidden contract");
        };
        forbidden.from[0].clone()
    }

    /// Each unit's label and the names of its members, and whether the
    /// selector matched.
    fn resolve(project: &Project, selector: &Selector) -> (Vec<(String, Vec<String>)>, bool) {
        let selection = project.select(selector);
        let units = selection
            .units
            .iter()
            .map(|unit| {
                let members = unit.members.iter().map(|id| project.graph.name(id).to_string());
                (unit.label.clone(), members.collect())
            })
            .collect();
        (units, selection.matched)
    }

    /// The labels of the units, and whether the selector matched.
    fn labels(project: &Project, selector: &Selector) -> (Vec<String>, bool) {
        let (units, matched) = resolve(project, selector);
        (units.into_iter().map(|(label, _)| label).collect(), matched)
    }

    /// The expected labels of a selection that matched.
    fn found(labels: &[&str]) -> (Vec<String>, bool) {
        (labels.iter().map(|label| label.to_string()).collect(), true)
    }

    #[test]
    fn a_directory_is_a_unit_of_its_files() {
        let project = project();
        let (units, matched) = resolve(&project, &selector("web/src/lib"));
        assert!(matched);
        assert_eq!(
            units,
            [(
                "web/src/lib".to_string(),
                vec!["web/src/lib/format.test.ts".to_string(), "web/src/lib/format.ts".to_string()]
            )]
        );
        let lib = project.graph.id("web/src/lib").unwrap();
        assert_eq!(project.select(&selector("web/src/lib")).units[0].root, lib);
    }

    #[test]
    fn path_globs_match_the_outermost_nodes_in_tree_order() {
        let project = project();
        assert_eq!(
            labels(&project, &selector("web/src/*")),
            found(&[
                "web/src/app",
                "web/src/env.d.ts",
                "web/src/lib",
                "web/src/types",
                "web/src/types.ts"
            ])
        );
        // `web/src` contains every other match.
        assert_eq!(labels(&project, &selector("web/**")), found(&["web/src"]));
        assert_eq!(
            labels(&project, &selector("**/*.tsx")),
            found(&[
                "packages/ui/button.tsx",
                "packages/ui/icons/star.tsx",
                "web/src/app/layout.tsx",
                "web/src/app/page.tsx",
            ])
        );
        assert_eq!(labels(&project, &selector("web/*/missing/*")), (Vec::new(), false));
        assert_eq!(labels(&project, &selector("missing/**")), (Vec::new(), false));
    }

    #[test]
    fn files_match_without_their_extension() {
        let project = project();
        // Both the directory and the file.
        assert_eq!(
            labels(&project, &selector("web/src/types")),
            found(&["web/src/types", "web/src/types.ts"])
        );
        // Not `format.test.ts`.
        assert_eq!(
            labels(&project, &selector("web/src/lib/format")),
            found(&["web/src/lib/format.ts"])
        );
        assert_eq!(labels(&project, &selector("web/src/env")), found(&["web/src/env.d.ts"]));
        assert_eq!(
            labels(&project, &selector("web/src/*/f*")),
            found(&["web/src/lib/format.test.ts", "web/src/lib/format.ts"])
        );
        // Only JavaScript and TypeScript extensions are removed.
        assert_eq!(labels(&project, &selector("packages/ui/README")), (Vec::new(), false));
        assert_eq!(
            labels(&project, &selector("packages/ui/README.md")),
            found(&["packages/ui/README.md"])
        );
        assert_eq!(labels(&project, &selector("web/src/missing")), (Vec::new(), false));
        assert_eq!(labels(&project, &selector("missing/types")), (Vec::new(), false));
    }

    #[test]
    fn strips_only_source_extensions() {
        assert_eq!(strip_source_extension("a/types.ts"), Some("a/types"));
        assert_eq!(strip_source_extension("a/types.d.ts"), Some("a/types"));
        assert_eq!(strip_source_extension("a/Button.tsx"), Some("a/Button"));
        assert_eq!(strip_source_extension("a/server.mjs"), Some("a/server"));
        assert_eq!(strip_source_extension("a/.eslintrc.cjs"), Some("a/.eslintrc"));
        assert_eq!(strip_source_extension("a/styles.css"), None);
        assert_eq!(strip_source_extension("a/Makefile"), None);
        assert_eq!(strip_source_extension("a/.ts"), None);
        assert_eq!(strip_source_extension(".d.ts"), None);
    }

    #[test]
    fn package_selectors_match_package_directories() {
        let project = project();
        let (units, matched) = resolve(&project, &selector("pkg:@acme/ui"));
        assert!(matched);
        assert_eq!(units[0].0, "@acme/ui");
        assert_eq!(
            units[0].1,
            [
                "packages/ui/README.md",
                "packages/ui/button.tsx",
                "packages/ui/icons/star.tsx",
                "packages/ui/project.ts",
            ]
        );

        // Packages without files have no unit, and units may be nested.
        assert_eq!(
            labels(&project, &selector("pkg:@acme/*")),
            found(&["@acme/api", "@acme/ui", "@acme/icons"])
        );
        assert_eq!(labels(&project, &selector("pkg:@acme/empty")), (Vec::new(), false));
    }

    #[test]
    fn only_the_exact_name_of_the_root_package_matches_it() {
        let project = project();
        assert!(!labels(&project, &selector("pkg:*")).0.contains(&"acme".to_string()));
        assert_eq!(labels(&project, &selector("pkg:acm?")), (Vec::new(), false));

        let selection = project.select(&selector("pkg:acme"));
        assert_eq!(selection.units.len(), 1);
        assert_eq!(selection.units[0].label, "acme");
        assert_eq!(selection.units[0].root, ModuleId::ROOT);
        assert_eq!(selection.units[0].members, project.graph.members(ModuleId::ROOT));
    }

    #[test]
    fn external_and_builtin_selectors_match_by_name() {
        let project = project();
        assert_eq!(labels(&project, &selector("npm:*")), found(&["npm:@acme/legacy", "npm:react"]));
        assert_eq!(labels(&project, &selector("npm:@acme/*")), found(&["npm:@acme/legacy"]));
        assert_eq!(labels(&project, &selector("node:*")), found(&["node:fs"]));
        assert_eq!(labels(&project, &selector("bun:*")), found(&["bun:sqlite"]));
        // Nothing imports it, which is not an error.
        assert_eq!(labels(&project, &selector("npm:sharp")), (Vec::new(), false));

        let (units, _) = resolve(&project, &selector("npm:react"));
        assert_eq!(units, [("npm:react".to_string(), vec!["npm:react".to_string()])]);
    }

    #[test]
    fn except_removes_members_from_each_unit() {
        let project = project();
        let (units, matched) = resolve(
            &project,
            &table(r#"{ path = "web/src/*", except = ["**/*.test.ts", "web/src/types"] }"#),
        );
        assert!(matched);
        let units: Vec<_> =
            units.iter().map(|(label, members)| (label.as_str(), members.len())).collect();
        assert_eq!(
            units,
            [
                ("web/src/app", 2),
                ("web/src/env.d.ts", 1),
                ("web/src/lib", 1),
                ("web/src/types", 0),
                ("web/src/types.ts", 0),
            ]
        );
    }

    #[test]
    fn a_directive_makes_one_unit_per_file_that_has_it() {
        let project = project();
        assert_eq!(
            labels(&project, &table(r#"{ path = "**", directive = "use client" }"#)),
            found(&[
                "packages/ui/button.tsx",
                "packages/ui/icons/star.tsx",
                "web/src/app/page.tsx"
            ])
        );
        // The nested packages share `star.tsx`, which is a unit once.
        assert_eq!(
            labels(&project, &table(r#"{ path = "pkg:@acme/*", directive = "use client" }"#)),
            found(&["packages/ui/button.tsx", "packages/ui/icons/star.tsx"])
        );
        // The base selector matched, so this isn't unmatched, just empty.
        assert_eq!(
            labels(&project, &table(r#"{ path = "web", directive = "use server" }"#)),
            (Vec::new(), true)
        );
        // `except` applies before the directive.
        assert_eq!(
            labels(
                &project,
                &table(
                    r#"{ path = "web", except = ["web/src/app/page.tsx"], directive = "use client" }"#
                )
            ),
            (Vec::new(), true)
        );
    }

    #[test]
    fn the_members_of_a_selection_are_those_of_every_unit() {
        let project = project();
        let selection = project.select(&selector("web/src/{lib,types}"));
        let members: Vec<_> =
            selection.members().iter().map(|id| project.graph.name(id).to_string()).collect();
        assert_eq!(
            members,
            [
                "web/src/lib/format.test.ts",
                "web/src/lib/format.ts",
                "web/src/types.ts",
                "web/src/types/user.ts",
            ]
        );
        assert!(Selection::default().members().is_empty());
    }
}
