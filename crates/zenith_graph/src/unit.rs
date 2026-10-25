//! Units: the labelled sets of nodes that contracts reason about.

use crate::{Graph, ModuleId, NodeKind, NodeSet};

/// A labelled set of the nodes that take part in imports, such as a
/// directory's files and the packages in it. Contracts reason about units, and
/// a [`UnitGraph`] aggregates the imports between them.
///
/// [`UnitGraph`]: crate::UnitGraph
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// How the unit is shown: the path of a directory or file, the name of a
    /// workspace package, or the name of an external package or builtin, e.g.
    /// `npm:react`.
    pub label: String,

    /// The node the unit stands for: a directory, a file, an external
    /// package or a builtin.
    pub root: ModuleId,

    /// The files and packages in the subtree of a directory, or the node
    /// itself.
    pub members: NodeSet,
}

impl Graph {
    /// The unit of the node `id`, labelled by its name.
    pub fn unit(&self, id: ModuleId) -> Unit {
        Unit { label: self.name(id).to_string(), root: id, members: self.members(id) }
    }

    /// The unit of the module that the node `id` is, or is part of: its
    /// [unit](Self::unit), unless a file and the directory beside it of the
    /// same name are one module, as Rust's `foo.rs` and `foo/` are, when it is
    /// the unit of the directory, labelled by its path, with the file among
    /// its members.
    pub fn module_unit(&self, id: ModuleId) -> Unit {
        let root = self.module_root(id);
        let mut unit = self.unit(root);
        if let Some(file) = self.module_file(root) {
            unit.members.insert(file);
        }
        unit
    }

    /// The [module units](Self::module_unit) of the nodes `ids`, each module
    /// once, in the order of the first of its nodes.
    pub fn module_units(&self, ids: impl IntoIterator<Item = ModuleId>) -> Vec<Unit> {
        let mut seen = NodeSet::new();
        (ids.into_iter())
            .filter(|id| seen.insert(self.module_root(*id)))
            .map(|id| self.module_unit(id))
            .collect()
    }

    /// The node that stands for the module of the node `id`: the directory
    /// that a file is one module with, as `src/crates` is for
    /// `src/crates.rs`, or else `id` itself.
    pub fn module_root(&self, id: ModuleId) -> ModuleId {
        let node = self.node(id);
        if node.kind() != NodeKind::File {
            return id;
        }
        let name = node.name();
        let stem = (self.directory_modules.iter())
            .find_map(|extension| name.strip_suffix(extension)?.strip_suffix('.'));
        let directory = stem.and_then(|stem| self.id(stem));
        directory
            .filter(|directory| self.node(*directory).kind() == NodeKind::Directory)
            .unwrap_or(id)
    }

    /// The file that is one module with the directory `id`, as
    /// `src/crates.rs` is with `src/crates`, if there is one.
    pub fn module_file(&self, id: ModuleId) -> Option<ModuleId> {
        if id == ModuleId::ROOT || self.node(id).kind() != NodeKind::Directory {
            return None;
        }
        let name = self.name(id);
        self.directory_modules.iter().find_map(|extension| {
            let file = self.id(&format!("{name}.{extension}"))?;
            (self.node(file).kind() == NodeKind::File).then_some(file)
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Graph, GraphBuilder,
        testing::{graph_of, id, names},
    };

    /// A graph of these files, where `rs` files are one module with the
    /// directory beside them of the same name.
    fn rust_graph(files: &[&str]) -> Graph {
        let mut builder = GraphBuilder::new();
        builder.set_directory_modules(vec!["rs"]);
        for file in files {
            builder.add_file(file).unwrap();
        }
        builder.finish()
    }

    #[test]
    fn a_unit_holds_the_files_of_its_node() {
        let graph =
            graph_of(&[("web/src/app.ts", "web/src/lib/a.ts"), ("web/src/app.ts", "npm:react")]);
        let unit = graph.unit(id(&graph, "web/src"));
        assert_eq!(unit.label, "web/src");
        assert_eq!(unit.root, id(&graph, "web/src"));
        assert_eq!(names(&graph, unit.members.iter()), ["web/src/app.ts", "web/src/lib/a.ts"]);

        let react = graph.unit(id(&graph, "npm:react"));
        assert_eq!(names(&graph, react.members.iter()), ["npm:react"]);
    }

    #[test]
    fn a_file_and_the_directory_beside_it_are_one_module_unit() {
        let graph = rust_graph(&[
            "src/crates.rs",
            "src/crates/cargo.rs",
            "src/crates/manifest.rs",
            "src/lib.rs",
            "src/web.ts",
            "src/web/app.ts",
        ]);
        for name in ["src/crates.rs", "src/crates"] {
            let unit = graph.module_unit(id(&graph, name));
            assert_eq!(unit.label, "src/crates");
            assert_eq!(unit.root, id(&graph, "src/crates"));
            assert_eq!(
                names(&graph, unit.members.iter()),
                ["src/crates.rs", "src/crates/cargo.rs", "src/crates/manifest.rs"]
            );
        }
        // A file without a directory beside it, and one of another language,
        // are units of their own.
        assert_eq!(
            graph.module_unit(id(&graph, "src/lib.rs")),
            graph.unit(id(&graph, "src/lib.rs"))
        );
        assert_eq!(
            graph.module_unit(id(&graph, "src/web.ts")),
            graph.unit(id(&graph, "src/web.ts"))
        );
        // Each module is one unit, however many of its nodes are given.
        let units = graph.module_units(graph.children(id(&graph, "src")).iter().copied());
        let labels: Vec<_> = units.iter().map(|unit| unit.label.as_str()).collect();
        assert_eq!(labels, ["src/crates", "src/lib.rs", "src/web", "src/web.ts"]);
    }
}
