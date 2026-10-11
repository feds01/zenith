//! Units: the labelled sets of nodes that contracts reason about.

use crate::{Graph, ModuleId, NodeSet};

/// A labelled set of the nodes that take part in imports, such as a
/// directory's files. Contracts reason about units, and a [`UnitGraph`]
/// aggregates the imports between them.
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

    /// The files in the subtree of a directory, or the node itself.
    pub members: NodeSet,
}

impl Graph {
    /// The unit of the node `id`, labelled by its name.
    pub fn unit(&self, id: ModuleId) -> Unit {
        Unit { label: self.name(id).to_string(), root: id, members: self.members(id) }
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::{graph_of, id, names};

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
}
