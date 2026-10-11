//! Helpers for the crate's unit tests.

use zenith_utils::location::ByteRange;

use crate::{Graph, GraphBuilder, ImportDetail, ImportKind, ModuleId, NodeHandle};

/// Build a graph from `(importer, imported)` pairs. Importers are files. An
/// imported name starting with `npm:` is an external package, one starting
/// with `node:` is a builtin, and anything else is a file. Each import is a
/// value import on its own line.
pub(crate) fn graph_of(imports: &[(&str, &str)]) -> Graph {
    let mut builder = GraphBuilder::new();
    for (index, (importer, imported)) in imports.iter().enumerate() {
        let importer = builder.add_file(importer).unwrap();
        let imported: NodeHandle = if imported.starts_with("npm:") {
            builder.add_external(imported).unwrap()
        } else if imported.starts_with("node:") {
            builder.add_builtin(imported).unwrap()
        } else {
            builder.add_file(imported).unwrap().into()
        };
        let line = u32::try_from(index).unwrap() + 1;
        builder.add_import(importer, imported, detail(line, ImportKind::Value));
    }
    builder.finish()
}

/// An import detail on `line`, with a span made up from the line.
pub(crate) fn detail(line: u32, kind: ImportKind) -> ImportDetail {
    ImportDetail {
        line,
        column: 1,
        range: ByteRange::new(line * 100, line * 100 + 10),
        specifier: format!("./module-{line}"),
        kind,
    }
}

/// The id of the node called `name`.
pub(crate) fn id(graph: &Graph, name: &str) -> ModuleId {
    graph.id(name).unwrap_or_else(|| panic!("no node `{name}`"))
}

/// The names of `ids`, in order.
pub(crate) fn names(graph: &Graph, ids: impl IntoIterator<Item = ModuleId>) -> Vec<&str> {
    ids.into_iter().map(|id| graph.name(id)).collect()
}
