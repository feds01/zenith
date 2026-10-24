//! The modules that suggestions are about: the workspace packages, or the
//! top-level directories when there are fewer than two packages.

use zenith_config::{Languages, Selector};
use zenith_diagnostics::message;
use zenith_graph::{Graph, ModuleId, NodeKind, NodeSet, UnitId};
use zenith_lang::Workspace;
use zenith_utils::glob;

use crate::Selection;

/// The selector of every workspace package.
const EVERY_PACKAGE: &str = "pkg:*";

/// The modules that suggestions are about.
pub(super) struct Modules {
    /// What they are.
    pub(super) kind: ModuleKind,

    /// The modules, in order of their directories, none inside another.
    pub(super) list: Vec<Module>,

    /// Whether they are every package that `pkg:*` selects, so that it can
    /// select them.
    every_package: bool,
}

/// What the modules that suggestions are about are.
#[derive(Debug, Clone, Copy)]
pub(super) enum ModuleKind {
    /// The workspace packages.
    Packages,

    /// The top-level directories.
    Directories,
}

impl ModuleKind {
    /// How contract ids name the modules.
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Packages => "packages",
            Self::Directories => "directories",
        }
    }

    /// How reasons name the modules.
    pub(super) fn noun(self) -> String {
        match self {
            Self::Packages => message!(contracts_suggest_packages),
            Self::Directories => message!(contracts_suggest_directories),
        }
    }

    /// How reasons name the modules in `dir`, or at the root if it is `""`.
    pub(super) fn siblings_noun(self, dir: &str) -> String {
        match (self, dir) {
            (Self::Packages, "") => message!(contracts_suggest_top_level_packages),
            (Self::Packages, dir) => message!(contracts_suggest_packages_in, dir),
            (Self::Directories, _) => message!(contracts_suggest_directories),
        }
    }
}

/// A workspace package or a top-level directory.
pub(super) struct Module {
    /// The selector that matches it, e.g. `pkg:@acme/ui` or `web`.
    selector: String,

    /// The directory that it is in, e.g. `packages`, or `""` at the root.
    parent: String,

    /// Its files and packages.
    pub(super) members: NodeSet,
}

impl Modules {
    /// The modules of the project of `graph` and `workspace`, whose
    /// selectors may use the schemes of the `languages`: the packages that
    /// `pkg:*` selects, but not those inside another, if there are two or
    /// more, or else the top-level directories, each as a path selector with
    /// its wildcards escaped.
    pub(super) fn of(graph: &Graph, workspace: &Workspace, languages: Languages) -> Self {
        let selector = Selector::parse(EVERY_PACKAGE, languages).expect("`pkg:*` is valid");
        let units = Selection::resolve(&selector, graph, workspace).units;
        let outermost = units.iter().filter(|unit| {
            !units
                .iter()
                .any(|outer| outer.root != unit.root && graph.is_in_subtree(unit.root, outer.root))
        });
        let packages: Vec<_> = outermost
            .map(|unit| {
                let dir = graph.name(unit.root);
                Module {
                    selector: format!("pkg:{}", unit.label),
                    parent: dir.rsplit_once('/').map_or("", |(parent, _)| parent).to_string(),
                    members: unit.members.clone(),
                }
            })
            .collect();
        if packages.len() >= 2 {
            let every_package = packages.len() == units.len();
            return Self { kind: ModuleKind::Packages, list: packages, every_package };
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
        Self { kind: ModuleKind::Directories, list: directories, every_package: false }
    }

    /// The selectors of every module: `pkg:*` when it selects exactly them,
    /// so that it covers the packages added later too, or else each module's
    /// own, in order.
    pub(super) fn all_selectors(&self) -> Vec<String> {
        match self.every_package {
            true => vec![EVERY_PACKAGE.to_string()],
            false => self.selectors((0..self.list.len()).map(UnitId::new)),
        }
    }

    /// The selectors of the modules of `units`, in their order.
    pub(super) fn selectors(&self, units: impl IntoIterator<Item = UnitId>) -> Vec<String> {
        units.into_iter().map(|unit| self.list[unit.index()].selector.clone()).collect()
    }

    /// The modules of each directory that has two or more, in order of the
    /// directories, as units.
    pub(super) fn siblings(&self) -> Vec<(&str, Vec<UnitId>)> {
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
