//! Contracts suggested from the import graph: those that the code, as it is,
//! already keeps.
//!
//! The suggestions are about the [modules](modules::Modules): the workspace
//! packages, or the top-level directories when there are fewer than two
//! packages. Each kind of contract is proposed by its own
//! [pass](passes::Pass) over the dependencies between them.
//!
//! Each contract that a pass proposes is written as TOML, parsed back as a
//! contract of the configuration, and checked against the same graph that it
//! was found in, so a suggestion that would be broken is never made. Nor is
//! one that a contract of the configuration already covers.

mod modules;
mod passes;

use std::{fmt, iter};

use modules::Modules;
use passes::{Context, PASSES};
use zenith_config::{Config, ContractConfig, ContractKind, Languages, Reexports, Selector};
use zenith_graph::{Graph, GraphView, ModuleId, UnitGraph};
use zenith_lang::Workspace;

use crate::{Contract, Exemptions, Graphs, Selection};

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
    /// `independence` contracts in order of their directories.
    ///
    /// A contract is not suggested when a contract of `config` of the same
    /// type already covers its modules. Its id is one that neither `config`
    /// nor an earlier suggestion has, with a number after it if it must be.
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

        // 3. Run each pass, and keep the contracts that it proposes that no contract of
        //    the configuration covers, and that the code keeps, under an id that is not
        //    taken yet.
        let mut suggestions: Vec<Self> = Vec::new();
        for candidate in PASSES.iter().flat_map(|pass| pass.propose(&context)) {
            let Some(contract) = candidate.parse(config, languages) else {
                continue;
            };
            let covered = config
                .contracts
                .iter()
                .any(|existing| covers(existing, &contract, graph, workspace));
            if covered || !is_kept(&contract, graph, workspace) {
                continue;
            }
            let taken = |id: &str| {
                config.contract(id).is_some()
                    || suggestions.iter().any(|suggestion| suggestion.id == id)
            };
            let numbered = (2..).map(|number| format!("{}-{number}", candidate.id));
            let id = iter::once(candidate.id.clone()).chain(numbered).find(|id| !taken(id));
            suggestions.push(candidate.with_id(id.expect("some number is free")));
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
        let mut table = Self::header(&id);
        write(&mut table);
        Self { id, reason, table }
    }

    /// The header of the table of the contract `id`, on a line of its own.
    fn header(id: &str) -> String {
        format!("[contracts.{id}]\n")
    }

    /// The suggestion under the id `id` instead.
    fn with_id(self, id: String) -> Self {
        let header = Self::header(&self.id);
        let body = self.table.strip_prefix(&header).expect("the table starts with its header");
        Self::new(id, self.reason, |table| table.push_str(body))
    }

    /// The contract, as a contract of `config`, which may enable the
    /// `languages`, or `None` if zenith would not accept it.
    fn parse(&self, config: &Config, languages: Languages) -> Option<ContractConfig> {
        let contracts = config.parse_contracts(&self.table, languages).ok()?;
        let [contract] = <[_; 1]>::try_from(contracts).ok()?;
        Some(contract)
    }
}

/// Whether the code in the project of `graph` and `workspace` keeps
/// `contract`. A contract whose selectors zenith would not accept is not
/// kept. `graph` is the one the contract sees, as it sees re-exports as the
/// contracts of the configuration do by default.
fn is_kept(contract: &ContractConfig, graph: &Graph, workspace: &Workspace) -> bool {
    let Ok(compiled) = Contract::compile(contract, graph, workspace) else {
        return false;
    };
    compiled.check(graph, workspace, &Exemptions::default()).is_kept()
}

/// Whether `existing` covers `contract`: it is of the same type, and selects
/// every module that `contract` does, in the project of `graph` and
/// `workspace`.
fn covers(
    existing: &ContractConfig,
    contract: &ContractConfig,
    graph: &Graph,
    workspace: &Workspace,
) -> bool {
    if existing.kind.type_name() != contract.kind.type_name() {
        return false;
    }
    let roots = |kind: &ContractKind| -> Vec<ModuleId> {
        module_selectors(kind)
            .into_iter()
            .flat_map(|selector| Selection::resolve(selector, graph, workspace).units)
            .map(|unit| unit.root)
            .collect()
    };
    let covered = roots(&existing.kind);
    roots(&contract.kind).iter().all(|root| covered.contains(root))
}

/// The selectors of the modules that a contract of the kinds that are
/// suggested is about: its `modules`, or the members of its layers. A
/// `layers` contract with containers is about the modules in each, so none of
/// the project's.
fn module_selectors(kind: &ContractKind) -> Vec<&Selector> {
    match kind {
        ContractKind::Acyclic(config) => config.modules.iter().collect(),
        ContractKind::Independence(config) => config.modules.iter().collect(),
        ContractKind::Layers(config) if config.containers.is_empty() => {
            config.layers.iter().flat_map(|layer| &layer.members).map(|m| &m.selector).collect()
        }
        _ => Vec::new(),
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

    use super::*;
    use crate::testing::{LANGUAGES, Project, values};

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
                "# These top-level directories stack in layers: each depends only on the layers below it.
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
        // Every package with files is a module, so `pkg:*` selects them. The
        // apps are not ordered, so they are one layer of independent siblings.
        let suggestions = suggest(&apps(&PACKAGES));
        let ids: Vec<_> = suggestions.iter().map(|suggestion| suggestion.id.as_str()).collect();
        assert_eq!(ids, ["packages-acyclic", "packages-layers", "apps-independent"]);
        assert_eq!(
            suggestions[0].table,
            "[contracts.packages-acyclic]
type = \"acyclic\"
modules = [\"pkg:*\"]
"
        );
        assert_eq!(
            suggestions[1].table,
            "[contracts.packages-layers]
type = \"layers\"
layers = [\"pkg:@s/admin | pkg:@s/web\", \"pkg:@s/ui\", \"pkg:@s/utils\"]
"
        );
        assert_eq!(
            suggestions[2].to_string(),
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
        assert_eq!(ids, ["packages-acyclic", "packages-layers", "apps-independent"]);
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

    /// The imports of a project whose top-level directories stack in layers,
    /// with `infra` and `lib`, which import nothing, side by side at the
    /// bottom.
    const STACKED: [(&str, &str); 3] =
        [("app/main.ts", "infra/db.ts"), ("app/main.ts", "ui/page.ts"), ("ui/page.ts", "lib/x.ts")];

    #[test]
    fn contracts_whose_modules_the_configuration_covers_are_not_suggested() {
        // The `layers` and `acyclic` contracts of the configuration select every
        // directory, under other ids and in other ways; `independence` has no
        // contract of its type.
        let covered =
            Project::new(&values(&STACKED), &[], include_str!("../fixtures/covering.toml"));
        assert!(ids(&covered).is_empty());
        // Without it, each is suggested.
        assert_eq!(ids(&project(&STACKED)), ["directories-acyclic", "directories-layers"]);
    }

    #[test]
    fn a_contract_that_covers_only_some_modules_leaves_its_id_to_another() {
        // `directories-layers` selects only some of the directories, so the
        // suggestion of all of them takes the next free id.
        let project = Project::new(&values(&STACKED), &[], include_str!("../fixtures/taken.toml"));
        let suggestions = suggest(&project);
        let ids: Vec<_> = suggestions.iter().map(|suggestion| suggestion.id.as_str()).collect();
        assert_eq!(ids, ["directories-acyclic", "directories-layers-3"]);
        assert_eq!(
            suggestions[1].table,
            "[contracts.directories-layers-3]
type = \"layers\"
layers = [\"app\", \"ui\", \"infra | lib\"]
"
        );
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
