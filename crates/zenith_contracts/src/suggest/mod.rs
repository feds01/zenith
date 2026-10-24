//! Contracts suggested from the import graph: those that the code, as it is,
//! already keeps.
//!
//! The suggestions are about the [modules](modules::Modules): the workspace
//! packages, or the top-level directories when there are fewer than two
//! packages. Each kind of contract is proposed by its own [`Pass`] over the
//! dependencies between them:
//!
//! - [`acyclic`], when they depend on each other in no cycle;
//! - [`layers`], when those that take part in a dependency stack in one strict
//!   order;
//! - [`independence`], for each set of siblings that don't depend on each
//!   other.
//!
//! A new kind of suggestion is a new pass, added to [`PASSES`]. Each contract
//! that a pass proposes is written as TOML, parsed back as a contract of the
//! configuration, and checked against the same graph that it was found in, so
//! a suggestion that would be broken is never made.

mod acyclic;
mod independence;
mod layers;
mod modules;

use std::fmt;

use modules::{ModuleKind, Modules};
use zenith_config::{Config, Languages, Reexports};
use zenith_graph::{Graph, GraphView, UnitGraph};
use zenith_lang::Workspace;
use zenith_pipeline::Graphs;

use crate::{Contract, Exemptions};

/// The passes, in the order that their suggestions are listed.
const PASSES: &[&dyn Pass] = &[&acyclic::Acyclic, &layers::Layers, &independence::Independence];

/// A pass that proposes contracts of one kind, from the modules and the
/// dependencies between them.
trait Pass {
    /// The contracts that the dependencies between the modules of `context`
    /// allow. They are checked before they are suggested.
    fn propose(&self, context: &Context) -> Vec<Suggestion>;
}

/// What the passes propose contracts from.
struct Context {
    /// The modules.
    modules: Modules,

    /// The dependencies between the modules, as units in their order.
    units: UnitGraph,

    /// The languages whose selector schemes the contracts may use.
    languages: Languages,
}

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
    /// whose id `config` or an earlier suggestion already has is not
    /// suggested.
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
        let modules = Modules::of(graph, workspace, languages);
        if modules.list.len() < 2 {
            return Vec::new();
        }

        // 2. Aggregate the imports between them, of the kinds that contracts see by
        //    default.
        let view = GraphView::without_kinds(graph, config.analysis.exclude_kinds);
        let members: Vec<_> = modules.list.iter().map(|module| module.members.clone()).collect();
        let units = UnitGraph::build(&view, &members);
        let context = Context { modules, units, languages };

        // 3. Run each pass, and keep the contracts that it proposes under an id that is
        //    not taken yet, and that the code keeps.
        let mut suggestions: Vec<Self> = Vec::new();
        for candidate in PASSES.iter().flat_map(|pass| pass.propose(&context)) {
            let taken = config.contract(&candidate.id).is_some()
                || suggestions.iter().any(|suggestion| suggestion.id == candidate.id);
            if !taken && candidate.is_kept(graph, workspace, config, languages) {
                suggestions.push(candidate);
            }
        }
        suggestions
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
        let project = Project::new(&imports, &[], include_str!("../fixtures/suggested.toml"));
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
        let project = Project::new(&imports, &[], include_str!("../fixtures/exclude-types.toml"));
        assert_eq!(ids(&project), ["directories-acyclic", "directories-layers"]);
        assert!(ids(&Project::new(&imports, &[], "")).is_empty());
    }
}
