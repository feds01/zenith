//! The `layers` contract type: higher layers may import lower layers, never
//! the reverse.

use zenith_config::{LayerMember, LayersConfig, SelectorTarget};
use zenith_graph::{Architecture, GraphView, LayerRef, Level, ModuleId, NodeKind, NodeSet};
use zenith_utils::{
    glob::{Glob, escape},
    location::ByteRange,
};

use crate::{
    Chain, ContractErrorKind, Selection, Violation, selection::UnitResolver, view::Statements,
};

/// A compiled `layers` contract.
#[derive(Debug)]
pub(crate) struct Layers {
    /// The layers of each container, or of the project root without
    /// containers.
    containers: Vec<Container>,

    /// Whether only direct imports count, rather than chains of imports.
    allow_indirect: bool,
}

/// The layers in one container, or in the project root.
#[derive(Debug)]
struct Container {
    /// The label of the container, or `None` for the project root.
    label: Option<String>,

    /// The layers, as an architecture whose levels are the layers of the
    /// contract and whose layers are their members.
    architecture: Architecture,

    /// The label of each member of each layer, by level, then member.
    labels: Vec<Vec<String>>,

    /// The children of the container that no layer declares, for an
    /// exhaustive contract.
    undeclared: Vec<ModuleId>,
}

impl Layers {
    /// Compile `config`, resolving its selectors with `units`. Layers that
    /// overlap, and members that match nothing unless they are optional, are
    /// errors.
    pub(crate) fn compile(config: &LayersConfig, units: &mut UnitResolver) -> Self {
        let containers = if config.containers.is_empty() {
            vec![Container::compile(config, None, units)]
        } else {
            let graph = units.graph();
            let mut containers = Vec::new();
            for (unit, range) in units.located_units(&config.containers) {
                if graph.node(unit.root).kind() == NodeKind::Directory {
                    containers.push(Container::compile(config, Some(unit.root), units));
                } else {
                    let kind = ContractErrorKind::NotADirectory { module: unit.label };
                    units.error(kind, range);
                }
            }
            containers
        };
        Self { containers, allow_indirect: config.allow_indirect_imports }
    }

    /// The illegal dependencies between the layers of each container, and
    /// the children that no layer declares, in the order of the containers.
    pub(crate) fn check(&self, view: &GraphView, statements: &Statements) -> Vec<Violation> {
        let mut violations = Vec::new();
        for container in &self.containers {
            let dependencies = if self.allow_indirect {
                view.find_illegal_direct_dependencies_for_layers(&container.architecture)
            } else {
                view.find_illegal_dependencies_for_layers(&container.architecture)
            };
            for dependency in dependencies {
                violations.push(Violation::Dependency {
                    importer: container.label(dependency.importer),
                    imported: container.label(dependency.imported),
                    chains: dependency
                        .routes
                        .iter()
                        .map(|route| Chain::from_route(view, statements, route))
                        .collect(),
                });
            }
            let label = container.label.as_deref().unwrap_or_default();
            violations.extend(container.undeclared.iter().map(|module| {
                Violation::UndeclaredModule { container: label.to_string(), module: *module }
            }));
        }
        violations
    }
}

/// A member of a layer, resolved in a container or in the project root.
///
/// For example, with `containers = ["apps/*"]`, the member `ui/*` resolved in
/// the container `apps/web` is labelled `apps/web/ui/*`. Its roots are the
/// nodes it matches, such as the directory `apps/web/ui/forms` and the file
/// `apps/web/ui/button.tsx`, and its members are `button.tsx` and every file
/// in `forms`.
struct ResolvedMember {
    /// How the member is shown: its selector, prefixed with the container.
    label: String,

    /// The nodes it matched, which the exhaustiveness check counts as
    /// declared.
    roots: Vec<ModuleId>,

    /// The members of the nodes it matched.
    members: NodeSet,
}

impl Container {
    /// The layers of `config` in the directory `container`, or in the project
    /// root if it's `None`. With a container, members are paths relative to
    /// it.
    fn compile(
        config: &LayersConfig,
        container: Option<ModuleId>,
        units: &mut UnitResolver,
    ) -> Self {
        let graph = units.graph();
        let container_name = container.map(|id| graph.name(id));

        // 1. Resolve the members of each layer. A member that matches nothing is empty,
        //    so that the layers are the same in every container.
        let resolved: Vec<Vec<ResolvedMember>> = config
            .layers
            .iter()
            .map(|layer| {
                let members = layer.members.iter();
                members
                    .map(|member| {
                        ResolvedMember::resolve(member, container_name, layer.range, units)
                    })
                    .collect()
            })
            .collect();

        // 2. Layers may not overlap.
        let all: Vec<_> = resolved
            .iter()
            .zip(&config.layers)
            .flat_map(|(members, layer)| members.iter().map(move |member| (member, layer.range)))
            .collect();
        'overlap: for (index, (first, _)) in all.iter().enumerate() {
            for (second, range) in &all[index + 1..] {
                if first.members.intersects(&second.members) {
                    let kind = ContractErrorKind::OverlappingModules {
                        first: first.label.clone(),
                        second: second.label.clone(),
                    };
                    units.error(kind, *range);
                    break 'overlap;
                }
            }
        }

        // 3. An exhaustive contract declares every child of the container, except the
        //    ignored ones, matched by their name with or without its extension. A file
        //    that is one module with the directory beside it is that directory's.
        let mut undeclared = Vec::new();
        if let Some(container) = container.filter(|_| config.exhaustive) {
            let roots: Vec<_> = all.iter().flat_map(|(member, _)| &member.roots).collect();
            for child in graph.children(container) {
                if graph.module_root(*child) != *child {
                    continue;
                }
                let declared = roots.iter().any(|root| {
                    graph.is_in_subtree(**root, *child) || graph.is_in_subtree(*child, **root)
                });
                let mut nodes = [Some(*child), graph.module_file(*child)].into_iter().flatten();
                let ignored = nodes.any(|node| {
                    let (name, stem) = (graph.node(node).base_name(), graph.base_stem(node));
                    config
                        .exhaustive_ignores
                        .iter()
                        .any(|pattern| pattern.is_match(name) || pattern.is_match(stem))
                });
                if !declared && !ignored {
                    undeclared.push(*child);
                }
            }
        }

        let labels = resolved
            .iter()
            .map(|members| members.iter().map(|member| member.label.clone()).collect())
            .collect();
        let levels = resolved.into_iter().zip(&config.layers).map(|(members, layer)| Level {
            layers: members.into_iter().map(|member| member.members).collect(),
            independent: layer.independent,
            closed: layer.closed,
        });
        Self {
            label: container_name.map(str::to_string),
            architecture: Architecture::new(levels),
            labels,
            undeclared,
        }
    }

    /// The label of `layer`.
    fn label(&self, layer: LayerRef) -> String {
        self.labels[layer.level.index()][layer.layer.index()].clone()
    }
}

impl ResolvedMember {
    /// Resolve `member`, written at `range`, in the `container`, or in the
    /// project root if it's `None`. A member that matches nothing, unless it's
    /// optional or an `npm:` or builtin selector, is an error.
    fn resolve(
        member: &LayerMember,
        container: Option<&str>,
        range: Option<ByteRange>,
        units: &mut UnitResolver,
    ) -> Self {
        let (label, found) = match container {
            Some(container) => (
                format!("{container}/{}", member.selector),
                Self::find_in_container(member, container, units),
            ),
            None => (member.selector.to_string(), Self::find(member, units)),
        };
        if found.is_none() && !member.optional && member.selector.must_match() {
            let selector = member.selector.to_string();
            let kind = match container {
                Some(container) => ContractErrorKind::MissingLayer {
                    member: selector,
                    container: container.to_string(),
                },
                None => ContractErrorKind::UnmatchedSelector { selector },
            };
            units.error(kind, range);
        }
        let (roots, members) = found.unwrap_or_default();
        Self { label, roots, members }
    }

    /// The roots and members of the units that `member` matches in the
    /// project, or `None` if it matches nothing.
    fn find(member: &LayerMember, units: &UnitResolver) -> Option<(Vec<ModuleId>, NodeSet)> {
        let selection = Selection::resolve(&member.selector, units.graph(), units.workspace());
        let roots = selection.units.iter().map(|unit| unit.root).collect();
        selection.matched.then(|| (roots, selection.members()))
    }

    /// The roots and members of the nodes that `member`, a path relative to
    /// the `container`, matches, or `None` if it matches nothing.
    fn find_in_container(
        member: &LayerMember,
        container: &str,
        units: &UnitResolver,
    ) -> Option<(Vec<ModuleId>, NodeSet)> {
        // The configuration only allows paths in the layers of containers.
        let SelectorTarget::Path(glob) = member.selector.target() else {
            return None;
        };
        let glob = Glob::new(&format!("{}/{}", escape(container), glob.as_str())).ok()?;
        let graph = units.graph();
        let found = graph.module_units(graph.find_paths(&glob));
        let roots: Vec<_> = found.iter().map(|unit| unit.root).collect();
        let mut members = NodeSet::new();
        for unit in &found {
            members.union_with(&unit.members);
        }
        (!roots.is_empty()).then_some((roots, members))
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ContractErrorKind, Violation,
        testing::{Project, values},
    };

    /// A `layers` contract `l` with the given options.
    fn contract(options: &str) -> String {
        format!("[contracts.l]\ntype = \"layers\"\n{options}\n")
    }

    #[test]
    fn lower_layers_may_not_import_higher_ones() {
        let project = Project::new(
            &values(&[
                ("src/high/h.ts", "src/low/l.ts"),
                ("src/low/l.ts", "src/high/h.ts"),
                // Through a layer: two dependencies, not three.
                ("src/low/m.ts", "src/mid/a.ts"),
                ("src/mid/a.ts", "src/high/h.ts"),
                // Through a module outside every layer.
                ("src/mid/b.ts", "src/util.ts"),
                ("src/util.ts", "src/high/i.ts"),
            ]),
            &[],
            &contract("layers = [\"src/high\", \"src/mid\", \"src/low\"]"),
        );
        assert_eq!(
            project.findings("l"),
            [
                "src/mid -> src/high: src/mid/a.ts -> src/high/h.ts",
                "src/mid -> src/high: src/mid/b.ts -> src/util.ts -> src/high/i.ts",
                "src/low -> src/high: src/low/l.ts -> src/high/h.ts",
                "src/low -> src/mid: src/low/m.ts -> src/mid/a.ts",
            ]
        );
    }

    #[test]
    fn siblings_are_independent_with_a_pipe_but_not_a_colon() {
        let imports = values(&[("src/a/x.ts", "src/b/y.ts")]);
        let independent = Project::new(&imports, &[], &contract("layers = [\"src/a | src/b\"]"));
        assert_eq!(independent.findings("l"), ["src/a -> src/b: src/a/x.ts -> src/b/y.ts"]);
        let dependent = Project::new(&imports, &[], &contract("layers = [\"src/a : src/b\"]"));
        assert!(dependent.check("l").is_kept());
    }

    #[test]
    fn a_closed_layer_may_not_be_bypassed() {
        let options =
            "layers = [\"src/high\", { layer = \"src/mid\", closed = true }, \"src/low\"]";
        let project = Project::new(
            &values(&[("src/high/h.ts", "src/low/l.ts"), ("src/mid/m.ts", "src/low/l.ts")]),
            &[],
            &contract(options),
        );
        // `src/mid` may import `src/low`, but `src/high` must go through it.
        assert_eq!(project.findings("l"), ["src/high -> src/low: src/high/h.ts -> src/low/l.ts"]);
    }

    #[test]
    fn a_chain_shows_its_other_heads_and_tails() {
        let project = Project::new(
            &values(&[
                ("low/a.ts", "utils.ts"),
                ("low/d.ts", "utils.ts"),
                ("utils.ts", "high/b.ts"),
                ("utils.ts", "high/c.ts"),
            ]),
            &[],
            &contract("layers = [\"high\", \"low\"]"),
        );
        let report = project.check("l");
        let [Violation::Dependency { chains, .. }] = report.violations.as_slice() else {
            panic!("expected one dependency, got {:?}", report.violations);
        };
        assert_eq!(project.findings("l"), ["low -> high: low/a.ts -> utils.ts -> high/b.ts"]);
        let name = |id| project.graph.name(id);
        let heads: Vec<_> = chains[0]
            .extra_heads
            .iter()
            .map(|link| (name(link.importer), name(link.imported)))
            .collect();
        let tails: Vec<_> = chains[0]
            .extra_tails
            .iter()
            .map(|link| (name(link.importer), name(link.imported)))
            .collect();
        assert_eq!(heads, [("low/d.ts", "utils.ts")]);
        assert_eq!(tails, [("utils.ts", "high/c.ts")]);
    }

    #[test]
    fn allowing_indirect_imports_only_counts_direct_ones() {
        let project = Project::new(
            &values(&[
                ("low/a.ts", "utils.ts"),
                ("utils.ts", "high/b.ts"),
                ("low/c.ts", "high/b.ts"),
            ]),
            &[],
            &contract("layers = [\"high\", \"low\"]\nallow_indirect_imports = true"),
        );
        assert_eq!(project.findings("l"), ["low -> high: low/c.ts -> high/b.ts"]);
    }

    #[test]
    fn chains_do_not_pass_through_boundaries() {
        let project = Project::with_directives(
            &values(&[
                ("low/a.ts", "lib/actions.ts"),
                ("lib/actions.ts", "high/b.ts"),
                ("low/c.ts", "lib/util.ts"),
                ("lib/util.ts", "high/b.ts"),
            ]),
            &[("lib/actions.ts", "use server")],
            include_str!("fixtures/layers-boundaries.toml"),
        );
        assert_eq!(project.findings("l"), ["low -> high: low/c.ts -> lib/util.ts -> high/b.ts"]);
    }

    #[test]
    fn containers_are_checked_on_their_own() {
        let project = Project::new(
            &values(&[
                ("domains/orders/types.ts", "domains/orders/api.ts"),
                // Across containers, which this contract doesn't check.
                ("domains/billing/types.ts", "domains/orders/api.ts"),
                ("domains/billing/api.ts", "domains/billing/hooks/use.ts"),
            ]),
            &[],
            &contract("containers = [\"domains/*\"]\nlayers = [\"(hooks)\", \"api\", \"types\"]"),
        );
        assert_eq!(
            project.findings("l"),
            [
                "domains/billing/api -> domains/billing/hooks: domains/billing/api.ts -> domains/billing/hooks/use.ts",
                "domains/orders/types -> domains/orders/api: domains/orders/types.ts -> domains/orders/api.ts",
            ]
        );
    }

    #[test]
    fn a_container_without_a_layer_that_is_not_optional_is_an_error() {
        let toml = contract("containers = [\"domains/*\"]\nlayers = [\"hooks\", \"api\"]");
        let project = Project::new(
            &values(&[
                ("domains/accounts/hooks/a.ts", "domains/accounts/api.ts"),
                ("domains/billing/api.ts", "x.ts"),
            ]),
            &[],
            &toml,
        );
        let errors = project.compile().unwrap_err();
        let messages: Vec<_> = errors.iter().map(|error| error.kind.to_string()).collect();
        assert_eq!(messages, ["`domains/billing` has no layer `hooks`"]);
        let range = errors[0].range.unwrap();
        assert_eq!(&toml[range.start() as usize..range.end() as usize], "\"hooks\"");
    }

    #[test]
    fn containers_must_be_directories() {
        let project = Project::new(
            &values(&[("src/a.ts", "src/b.ts")]),
            &[],
            &contract("containers = [\"src/a.ts\"]\nlayers = [\"x\"]"),
        );
        let errors = project.compile().unwrap_err();
        assert_eq!(
            errors[0].kind,
            ContractErrorKind::NotADirectory { module: "src/a.ts".to_string() }
        );
    }

    #[test]
    fn layers_may_not_overlap() {
        let project = Project::new(
            &values(&[("src/a/x.ts", "src/b/y.ts")]),
            &[],
            &contract("layers = [\"src\", \"src/a\"]"),
        );
        let errors = project.compile().unwrap_err();
        assert_eq!(
            errors[0].kind,
            ContractErrorKind::OverlappingModules {
                first: "src".to_string(),
                second: "src/a".to_string()
            }
        );
    }

    #[test]
    fn exhaustive_layers_declare_every_child_of_their_containers() {
        let project = Project::new(
            &values(&[
                ("domains/orders/api.ts", "domains/orders/types.ts"),
                ("domains/orders/ui.ts", "domains/orders/api.ts"),
                ("domains/orders/index.ts", "domains/orders/ui.ts"),
                ("domains/orders/utils.ts", "domains/orders/types.ts"),
            ]),
            &[],
            &contract(
                "containers = [\"domains/*\"]\nlayers = [\"ui\", \"api\", \"types\"]\nexhaustive = true\nexhaustive_ignores = [\"index\"]",
            ),
        );
        assert_eq!(
            project.findings("l"),
            ["domains/orders: `domains/orders/utils.ts` is not a layer"]
        );
    }

    #[test]
    fn a_rust_module_and_its_directory_are_one_layer() {
        // `crates.rs` and `crates/` are the module `crates`, so the member
        // `crates` declares both, and its own imports are within the layer.
        let project = Project::new(
            &values(&[
                ("crates/lang/src/crates.rs", "crates/lang/src/crates/manifest.rs"),
                ("crates/lang/src/crates/manifest.rs", "crates/lang/src/model.rs"),
                ("crates/lang/src/model.rs", "crates/lang/src/crates.rs"),
                ("crates/lang/src/lib.rs", "crates/lang/src/crates.rs"),
            ]),
            &[],
            include_str!("fixtures/layers-rust-modules.toml"),
        );
        assert_eq!(
            project.findings("l"),
            [
                "crates/lang/src/model -> crates/lang/src/crates: crates/lang/src/model.rs -> crates/lang/src/crates.rs"
            ]
        );
    }

    #[test]
    fn layers_can_be_packages_and_externals() {
        let project = Project::new(
            &values(&[("lib/a.ts", "npm:react"), ("app/b.ts", "lib/a.ts")]),
            &[],
            &contract("layers = [\"app\", \"npm:react\", \"lib\", \"npm:left-pad\"]"),
        );
        assert_eq!(project.findings("l"), ["lib -> npm:react: lib/a.ts -> npm:react"]);
    }
}
