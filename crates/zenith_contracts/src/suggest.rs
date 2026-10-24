//! Contracts suggested from the import graph: those that the code, as it is,
//! already keeps.
//!
//! The suggestions are about the workspace packages, or the top-level
//! directories when there are fewer than two packages:
//!
//! - `acyclic`, when they depend on each other in no cycle, with the packages
//!   written as `pkg:*` when that selects just them;
//! - `layers`, when those that take part in a dependency stack in one strict
//!   order, each depending on the next;
//! - `independence`, for each set of siblings, the modules in one directory,
//!   none of which depends on another.
//!
//! Each contract is written as TOML, parsed back as a contract of the
//! configuration, and checked against the same graph that it was found in, so
//! a suggestion that would be broken is never made.

use std::fmt;

use zenith_config::{Config, ContractConfig, Languages, Layer, Reexports, Selector};
use zenith_diagnostics::message;
use zenith_graph::{Graph, GraphView, ModuleId, NodeKind, NodeSet, UnitGraph, UnitId};
use zenith_lang::Workspace;
use zenith_pipeline::Graphs;
use zenith_utils::{glob, toml_writing::array};

use crate::{Contract, Exemptions, Selection};

/// A contract that the code already keeps, suggested for the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The id of the contract.
    pub id: String,

    /// Why the contract is suggested, as a sentence.
    pub reason: String,

    /// The contract's table, as TOML.
    pub table: String,
}

impl Suggestion {
    /// Suggest the contracts that the code keeps, in the project of `graphs`
    /// and `workspace`, when they are added to `config`, which may enable the
    /// `languages`: an `acyclic` contract, then a `layers` contract, then the
    /// `independence` contracts in order of their directories. A contract
    /// whose id `config` already has is not suggested.
    ///
    /// Panics if the contracts of `config` see through re-exports by default,
    /// but `graphs` has no graph through them: see
    /// [`Suggestion::needs_graph_through_reexports`].
    pub fn find(
        graphs: Graphs,
        workspace: &Workspace,
        config: &Config,
        languages: Languages,
    ) -> Vec<Self> {
        // 1. Take the workspace packages as the modules, or else the top-level
        //    directories, in the graph that the contracts of the configuration see by
        //    default.
        let graph = graphs.seeing(config.analysis.reexports);
        let modules = Modules::of(graph, workspace);
        if modules.list.len() < 2 {
            return Vec::new();
        }

        // 2. Aggregate the imports between them, of the kinds that contracts see by
        //    default.
        let view = GraphView::without_kinds(graph, config.analysis.exclude_kinds);
        let members: Vec<_> = modules.list.iter().map(|module| module.members.clone()).collect();
        let units = UnitGraph::build(&view, &members);

        // 3. Propose the contracts that the dependencies between them allow.
        let mut candidates = Vec::new();
        if units.nominate_cycle_breakers().is_empty() {
            let reason = message!(contracts_suggest_acyclic, modules = modules.kind.noun());
            let selectors = modules.all_selectors(graph, workspace, languages);
            let id = format!("{}-acyclic", modules.kind.id());
            candidates.push(Self::new(id, reason, |table| {
                table.push_str("type = \"acyclic\"\n");
                table.push_str(&array("modules", &selectors));
            }));
        }
        if let Some(order) = units.strict_order() {
            let selectors = modules.selectors(order);
            if selectors.iter().all(|selector| is_plain_layer(selector, languages)) {
                let reason = message!(contracts_suggest_layers, modules = modules.kind.noun());
                let id = format!("{}-layers", modules.kind.id());
                candidates.push(Self::new(id, reason, |table| {
                    table.push_str("type = \"layers\"\n");
                    table.push_str(&array("layers", &selectors));
                }));
            }
        }
        for (dir, siblings) in modules.siblings() {
            let independent = siblings.iter().all(|unit| {
                let reached = units.reachable_from(*unit);
                !reached.iter().any(|other| other != unit && siblings.contains(other))
            });
            if !independent {
                continue;
            }
            let id = match (modules.kind, dir) {
                (ModuleKind::Packages, "") => "top-level-packages-independent".to_string(),
                (ModuleKind::Packages, dir) => format!("{}-independent", slug(dir)),
                (ModuleKind::Directories, _) => "directories-independent".to_string(),
            };
            if candidates.iter().any(|candidate| candidate.id == id) {
                continue;
            }
            let noun = modules.kind.siblings_noun(dir);
            let reason = message!(contracts_suggest_independence, modules = noun);
            let selectors = modules.selectors(siblings);
            candidates.push(Self::new(id, reason, |table| {
                table.push_str("type = \"independence\"\n");
                table.push_str(&array("modules", &selectors));
            }));
        }

        // 4. Keep only the contracts that the configuration doesn't have yet, and that
        //    the code keeps.
        candidates.retain(|candidate| {
            config.contract(&candidate.id).is_none()
                && candidate.is_kept(graph, workspace, config, languages)
        });
        candidates
    }

    /// Whether the suggested contracts see through re-exports, so are found
    /// and checked in the graph through them: when the contracts of `config`
    /// do by default.
    pub fn needs_graph_through_reexports(config: &Config) -> bool {
        config.analysis.reexports == Reexports::Transparent
    }

    /// The suggestion of the contract `id`, for `reason`, whose table `write`
    /// fills in below its header.
    fn new(id: String, reason: String, write: impl FnOnce(&mut String)) -> Self {
        let mut table = format!("[contracts.{id}]\n");
        write(&mut table);
        Self { id, reason, table }
    }

    /// Whether the code in the project of `graph` and `workspace` keeps the
    /// contract, when it is added to `config`, which may enable the
    /// `languages`. A contract that zenith would not accept is not kept.
    /// `graph` is the one the contract sees, as it sees re-exports as the
    /// contracts of `config` do by default.
    fn is_kept(
        &self,
        graph: &Graph,
        workspace: &Workspace,
        config: &Config,
        languages: Languages,
    ) -> bool {
        let Ok(contracts) = config.parse_contracts(&self.table, languages) else {
            return false;
        };
        let [contract] = contracts.as_slice() else {
            return false;
        };
        let Ok(compiled) = Contract::compile(contract, graph, workspace) else {
            return false;
        };
        compiled.check(graph, workspace, &Exemptions::default()).is_kept()
    }
}

/// The suggestion as a comment with its reason, then its table.
impl fmt::Display for Suggestion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "# {}\n{}", self.reason, self.table)
    }
}

/// The modules that suggestions are about.
struct Modules {
    /// What they are.
    kind: ModuleKind,

    /// The modules, in order of their directories, none inside another.
    list: Vec<Module>,
}

/// What the modules that suggestions are about are.
#[derive(Debug, Clone, Copy)]
enum ModuleKind {
    /// The workspace packages.
    Packages,

    /// The top-level directories.
    Directories,
}

impl ModuleKind {
    /// How contract ids name the modules.
    fn id(self) -> &'static str {
        match self {
            Self::Packages => "packages",
            Self::Directories => "directories",
        }
    }

    /// How reasons name the modules.
    fn noun(self) -> String {
        match self {
            Self::Packages => message!(contracts_suggest_packages),
            Self::Directories => message!(contracts_suggest_directories),
        }
    }

    /// How reasons name the modules in `dir`, or at the root if it is `""`.
    fn siblings_noun(self, dir: &str) -> String {
        match (self, dir) {
            (Self::Packages, "") => message!(contracts_suggest_top_level_packages),
            (Self::Packages, dir) => message!(contracts_suggest_packages_in, dir),
            (Self::Directories, _) => message!(contracts_suggest_directories),
        }
    }
}

/// A workspace package or a top-level directory.
struct Module {
    /// The selector that matches it, e.g. `pkg:@acme/ui` or `web`.
    selector: String,

    /// The directory that it is in, e.g. `packages`, or `""` at the root.
    parent: String,

    /// Its files and packages.
    members: NodeSet,
}

impl Modules {
    /// The modules of the project of `graph` and `workspace`: its workspace
    /// packages with files, but not those inside another, if there are two or
    /// more, or else its top-level directories, each as a path selector with
    /// its wildcards escaped.
    fn of(graph: &Graph, workspace: &Workspace) -> Self {
        let mut packages: Vec<Module> = Vec::new();
        for package in &workspace.packages {
            let Some(id) = graph.id(&package.dir) else {
                continue;
            };
            let nested = workspace.packages.iter().any(|outer| {
                package.dir.strip_prefix(&outer.dir).is_some_and(|rest| rest.starts_with('/'))
            });
            if nested {
                continue;
            }
            let parent = package.dir.rsplit_once('/').map_or("", |(parent, _)| parent);
            packages.push(Module {
                selector: format!("pkg:{}", package.name),
                parent: parent.to_string(),
                members: graph.members(id),
            });
        }
        if packages.len() >= 2 {
            return Self { kind: ModuleKind::Packages, list: packages };
        }

        let directories = graph
            .children(ModuleId::ROOT)
            .iter()
            .filter(|child| graph.node(**child).kind() == NodeKind::Directory)
            .map(|child| Module {
                selector: glob::escape(graph.name(*child)),
                parent: String::new(),
                members: graph.members(*child),
            })
            .collect();
        Self { kind: ModuleKind::Directories, list: directories }
    }

    /// The selectors of every module: `pkg:*` for the packages when it
    /// selects exactly them, so that it covers the packages added later too,
    /// or else each module's own, in order.
    fn all_selectors(
        &self,
        graph: &Graph,
        workspace: &Workspace,
        languages: Languages,
    ) -> Vec<String> {
        const EVERY_PACKAGE: &str = "pkg:*";
        if let ModuleKind::Packages = self.kind {
            let selector = Selector::parse(EVERY_PACKAGE, languages).expect("`pkg:*` is valid");
            let units = Selection::resolve(&selector, graph, workspace).units;
            if units.iter().map(|unit| &unit.members).eq(self.list.iter().map(|m| &m.members)) {
                return vec![EVERY_PACKAGE.to_string()];
            }
        }
        self.selectors((0..self.list.len()).map(UnitId::new))
    }

    /// The selectors of the modules of `units`, in their order.
    fn selectors(&self, units: impl IntoIterator<Item = UnitId>) -> Vec<String> {
        units.into_iter().map(|unit| self.list[unit.index()].selector.clone()).collect()
    }

    /// The modules of each directory that has two or more, in order of the
    /// directories, as units.
    fn siblings(&self) -> Vec<(&str, Vec<UnitId>)> {
        let mut groups: Vec<(&str, Vec<UnitId>)> = Vec::new();
        for (index, module) in self.list.iter().enumerate() {
            let unit = UnitId::new(index);
            match groups.iter_mut().find(|(dir, _)| *dir == module.parent) {
                Some((_, units)) => units.push(unit),
                None => groups.push((&module.parent, vec![unit])),
            }
        }
        groups.retain(|(_, units)| units.len() >= 2);
        groups.sort_by(|a, b| a.0.cmp(b.0));
        groups
    }
}

/// Whether `selector`, written as a layer, is a layer of just that selector,
/// rather than of the members or optional member that `|`, ` : ` or
/// parentheses in a directory's name would make of it.
fn is_plain_layer(selector: &str, languages: Languages) -> bool {
    Layer::parse(selector, false, languages).is_ok_and(|layer| {
        matches!(layer.members.as_slice(), [member]
            if !member.optional && member.selector.as_str() == selector)
    })
}

/// `dir` as part of a contract id, e.g. `packages-libs` for `packages/libs`:
/// every character that ids can't have becomes a `-`.
fn slug(dir: &str) -> String {
    dir.chars().map(|c| if ContractConfig::is_id_char(c) { c } else { '-' }).collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use zenith_graph::ImportKind;
    use zenith_lang::WorkspacePackage;
    use zenith_pipeline::LANGUAGES;

    use super::*;
    use crate::testing::{Project, values};

    /// A project of the value imports `pairs`, without a configuration.
    fn project(pairs: &[(&str, &str)]) -> Project {
        Project::new(&values(pairs), &[], "")
    }

    /// The project with the workspace packages of `(name, dir)`.
    fn with_packages(mut project: Project, packages: &[(&str, &str)]) -> Project {
        project.workspace.packages = packages
            .iter()
            .map(|(name, dir)| WorkspacePackage {
                name: name.to_string(),
                dir: dir.to_string(),
                path: PathBuf::from("/repo").join(dir),
            })
            .collect();
        project
    }

    /// The suggestions for `project`.
    fn suggest(project: &Project) -> Vec<Suggestion> {
        Suggestion::find((&project.graph).into(), &project.workspace, &project.config, LANGUAGES)
    }

    /// The ids of the suggestions for `project`.
    fn ids(project: &Project) -> Vec<String> {
        suggest(project).into_iter().map(|suggestion| suggestion.id).collect()
    }

    #[test]
    fn top_level_directories_that_stack_are_acyclic_layers() {
        let project = project(&[
            ("app/main.ts", "domain/order.ts"),
            ("domain/order.ts", "infra/db.ts"),
            ("app/main.ts", "infra/db.ts"),
            ("scripts/seed.ts", "npm:pg"),
        ]);
        let shown: Vec<_> = suggest(&project).iter().map(Suggestion::to_string).collect();
        assert_eq!(
            shown,
            [
                "# The top-level directories depend on each other in no cycle.
[contracts.directories-acyclic]
type = \"acyclic\"
modules = [\"app\", \"domain\", \"infra\", \"scripts\"]
",
                "# These top-level directories stack in one order: each depends on the next.
[contracts.directories-layers]
type = \"layers\"
layers = [\"app\", \"domain\", \"infra\"]
",
            ]
        );
    }

    #[test]
    fn a_suggestion_that_would_be_broken_is_never_made() {
        // `infra` reaches `app` through a file outside every directory, which
        // breaks the layers, but makes no cycle between the directories.
        let project = project(&[
            ("app/main.ts", "domain/order.ts"),
            ("domain/order.ts", "infra/db.ts"),
            ("infra/db.ts", "config.ts"),
            ("config.ts", "app/main.ts"),
        ]);
        assert_eq!(ids(&project), ["directories-acyclic"]);
    }

    /// Two apps that import `ui`, which imports `utils`, with a package
    /// inside `ui`, and the workspace packages of `(name, dir)`.
    fn apps(packages: &[(&str, &str)]) -> Project {
        let project = project(&[
            ("apps/web/page.ts", "packages/ui/button.ts"),
            ("apps/admin/page.ts", "packages/ui/button.ts"),
            ("apps/admin/page.ts", "packages/utils/format.ts"),
            ("packages/ui/button.ts", "packages/utils/format.ts"),
            ("packages/ui/icons/star.ts", "npm:react"),
        ]);
        with_packages(project, packages)
    }

    /// The packages of [`apps`], with one without files, but not the one
    /// inside `ui`.
    const PACKAGES: [(&str, &str); 5] = [
        ("@s/admin", "apps/admin"),
        ("@s/web", "apps/web"),
        ("@s/empty", "packages/empty"),
        ("@s/ui", "packages/ui"),
        ("@s/utils", "packages/utils"),
    ];

    #[test]
    fn sibling_packages_that_do_not_depend_on_each_other_are_independent() {
        // The apps are not ordered, so the packages don't stack. Every package
        // with files is a module, so `pkg:*` selects them.
        let suggestions = suggest(&apps(&PACKAGES));
        let ids: Vec<_> = suggestions.iter().map(|suggestion| suggestion.id.as_str()).collect();
        assert_eq!(ids, ["packages-acyclic", "apps-independent"]);
        assert_eq!(
            suggestions[0].table,
            "[contracts.packages-acyclic]
type = \"acyclic\"
modules = [\"pkg:*\"]
"
        );
        assert_eq!(
            suggestions[1].to_string(),
            "# The workspace packages in `apps` do not depend on each other.
[contracts.apps-independent]
type = \"independence\"
modules = [\"pkg:@s/admin\", \"pkg:@s/web\"]
"
        );
    }

    #[test]
    fn packages_inside_another_are_left_out_and_the_rest_are_listed() {
        // `pkg:*` would select the package inside `ui` too, which overlaps it.
        let project = apps(&[&PACKAGES[..], &[("@s/icons", "packages/ui/icons")]].concat());
        let suggestions = suggest(&project);
        let ids: Vec<_> = suggestions.iter().map(|suggestion| suggestion.id.as_str()).collect();
        assert_eq!(ids, ["packages-acyclic", "apps-independent"]);
        assert_eq!(
            suggestions[0].table,
            "[contracts.packages-acyclic]
type = \"acyclic\"
modules = [\"pkg:@s/admin\", \"pkg:@s/web\", \"pkg:@s/ui\", \"pkg:@s/utils\"]
"
        );
    }

    #[test]
    fn contracts_that_the_configuration_has_are_not_suggested_again() {
        let imports = values(&[("app/main.ts", "domain/order.ts")]);
        let project = Project::new(&imports, &[], include_str!("fixtures/suggested.toml"));
        assert_eq!(ids(&project), ["directories-layers"]);
    }

    #[test]
    fn directories_are_selected_by_their_names_as_written() {
        // `[locale]` needs no escaping, and `{legacy}` is escaped. They stack
        // on `(shared)`, but it would be an optional layer, so there are none.
        let project = project(&[
            ("[locale]/page.ts", "{legacy}/old.ts"),
            ("{legacy}/old.ts", "(shared)/util.ts"),
        ]);
        let tables: Vec<_> = suggest(&project).into_iter().map(|s| s.table).collect();
        assert_eq!(
            tables,
            ["[contracts.directories-acyclic]
type = \"acyclic\"
modules = [\"(shared)\", \"[locale]\", '\\{legacy\\}']
"]
        );
        assert!(is_plain_layer("[locale]", LANGUAGES));
        assert!(!is_plain_layer("(shared)", LANGUAGES));
        assert!(!is_plain_layer("a | b", LANGUAGES));
    }

    #[test]
    fn modules_in_a_cycle_get_no_suggestion() {
        assert!(ids(&project(&[("a/x.ts", "b/y.ts"), ("b/y.ts", "a/x.ts")])).is_empty());
        // Nor does a single module.
        assert!(ids(&project(&[("a/x.ts", "npm:react")])).is_empty());
    }

    #[test]
    fn imports_of_the_kinds_that_contracts_exclude_by_default_do_not_count() {
        let imports =
            [("a/x.ts", "b/y.ts", ImportKind::Value), ("b/y.ts", "a/x.ts", ImportKind::Type)];
        let project = Project::new(&imports, &[], include_str!("fixtures/exclude-types.toml"));
        assert_eq!(ids(&project), ["directories-acyclic", "directories-layers"]);
        assert!(ids(&Project::new(&imports, &[], "")).is_empty());
    }
}
