//! The graph data structure: the module tree, plus the imports between nodes.

use rustc_hash::FxHashMap;

use crate::{ImportKind, KindSet, NodeSet};

/// An opaque handle to a node in a [Graph].
///
/// Ids are assigned in alphabetical order of node names when the graph is
/// built, so sorting by id is the same as sorting by name. This keeps every
/// traversal, and therefore every report, deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModuleId(u32);

impl ModuleId {
    /// The root directory of the project.
    pub const ROOT: ModuleId = ModuleId(0);

    /// Create an id from a raw index.
    pub fn from_index(index: usize) -> Self {
        Self(index as u32)
    }

    /// The raw index of the id.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// What a node in the graph represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// A directory inside the project. Directories stand for their whole
    /// subtree and never have imports of their own.
    Directory,
    /// A file inside the project.
    File,
    /// An external package, e.g. `npm:react`.
    External,
    /// A runtime builtin, e.g. `node:fs`.
    Builtin,
}

/// A node of the module tree, or an external package or builtin.
#[derive(Debug, Clone)]
pub struct Node {
    /// The node's id: a POSIX path relative to the root for internal nodes
    /// (`""` is the root itself), or `npm:<name>` / `node:<name>`.
    pub name: String,

    /// What the node represents.
    pub kind: NodeKind,

    /// The parent directory. `None` for the root and for external nodes.
    pub parent: Option<ModuleId>,

    /// Child nodes, sorted by name.
    pub children: Vec<ModuleId>,

    /// Whether the file was parsed for imports. Files that are only import
    /// targets (assets, `dist/` output) are not scanned.
    pub scanned: bool,

    /// The directive prologue of a scanned file, e.g. `["use client"]`.
    pub directives: Vec<String>,

    /// The position of the node in a preorder traversal of the module tree,
    /// and the end (exclusive) of its subtree in that traversal.
    pre: u32,
    post: u32,
}

impl Node {
    /// Whether this node is part of the project's module tree.
    pub fn is_internal(&self) -> bool {
        matches!(self.kind, NodeKind::Directory | NodeKind::File)
    }

    /// The last segment of the node's name, e.g. `api.ts` for `src/api.ts`.
    pub fn base_name(&self) -> &str {
        if self.is_internal() {
            self.name.rsplit('/').next().unwrap_or(&self.name)
        } else {
            &self.name
        }
    }
}

/// One import statement or expression that creates an edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDetail {
    /// The 1-based line of the import.
    pub line: u32,
    /// The 1-based column of the import.
    pub column: u32,
    /// The byte span of the import in the importing file.
    pub start: u32,
    pub end: u32,
    /// The specifier as written in the source, e.g. `@/domains/campaigns`.
    pub specifier: String,
    /// How the import is made.
    pub kind: ImportKind,
}

/// The module tree and import graph of a project.
#[derive(Debug, Clone, Default)]
pub struct Graph {
    nodes: Vec<Node>,
    by_name: FxHashMap<String, ModuleId>,
    imports: Vec<Vec<ModuleId>>,
    importers: Vec<Vec<ModuleId>>,
    details: FxHashMap<(ModuleId, ModuleId), Vec<ImportDetail>>,
    kinds: FxHashMap<(ModuleId, ModuleId), KindSet>,
    preorder: Vec<ModuleId>,
}

impl Graph {
    /// The number of nodes in the graph.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the graph has no nodes (it always has at least the root).
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Get a node.
    pub fn node(&self, id: ModuleId) -> &Node {
        &self.nodes[id.index()]
    }

    /// The name of a node.
    pub fn name(&self, id: ModuleId) -> &str {
        &self.nodes[id.index()].name
    }

    /// Look up a node by name.
    pub fn id(&self, name: &str) -> Option<ModuleId> {
        self.by_name.get(name).copied()
    }

    /// Iterate over every node id, in alphabetical order.
    pub fn ids(&self) -> impl Iterator<Item = ModuleId> + '_ {
        (0..self.nodes.len()).map(ModuleId::from_index)
    }

    /// The modules directly imported by `id`, in ascending id order.
    pub fn imports_of(&self, id: ModuleId) -> &[ModuleId] {
        &self.imports[id.index()]
    }

    /// The modules that directly import `id`, in ascending id order.
    pub fn importers_of(&self, id: ModuleId) -> &[ModuleId] {
        &self.importers[id.index()]
    }

    /// Whether `importer` directly imports `imported`.
    pub fn has_import(&self, importer: ModuleId, imported: ModuleId) -> bool {
        self.imports[importer.index()].binary_search(&imported).is_ok()
    }

    /// The details of every import from `importer` to `imported`, in source
    /// order.
    pub fn details(&self, importer: ModuleId, imported: ModuleId) -> &[ImportDetail] {
        self.details.get(&(importer, imported)).map_or(&[], Vec::as_slice)
    }

    /// The import kinds of every import from `importer` to `imported`.
    pub fn kinds(&self, importer: ModuleId, imported: ModuleId) -> KindSet {
        self.kinds.get(&(importer, imported)).copied().unwrap_or_default()
    }

    /// Iterate over every `(importer, imported)` pair.
    pub fn edges(&self) -> impl Iterator<Item = (ModuleId, ModuleId)> + '_ {
        self.imports.iter().enumerate().flat_map(|(index, imported)| {
            imported.iter().map(move |imported| (ModuleId::from_index(index), *imported))
        })
    }

    /// The number of `(importer, imported)` pairs.
    pub fn edge_count(&self) -> usize {
        self.imports.iter().map(Vec::len).sum()
    }

    /// The number of individual import statements.
    pub fn import_count(&self) -> usize {
        self.details.values().map(Vec::len).sum()
    }

    /// The children of a node, sorted by name.
    pub fn children(&self, id: ModuleId) -> &[ModuleId] {
        &self.nodes[id.index()].children
    }

    /// The node itself followed by all of its descendants, in preorder. For
    /// external nodes this is just the node.
    pub fn subtree(&self, id: ModuleId) -> &[ModuleId] {
        let node = &self.nodes[id.index()];
        &self.preorder[node.pre as usize..node.post as usize]
    }

    /// Whether `id` is `ancestor` or one of its descendants.
    pub fn is_in_subtree(&self, id: ModuleId, ancestor: ModuleId) -> bool {
        let node = &self.nodes[id.index()];
        let ancestor = &self.nodes[ancestor.index()];
        ancestor.pre <= node.pre && node.pre < ancestor.post
    }

    /// The set of nodes that can carry imports in the subtree of `id`: every
    /// file, external and builtin node in it.
    pub fn members(&self, id: ModuleId) -> NodeSet {
        self.subtree(id)
            .iter()
            .copied()
            .filter(|member| self.nodes[member.index()].kind != NodeKind::Directory)
            .collect()
    }
}

/// Incrementally builds a [Graph].
#[derive(Debug, Default)]
pub struct GraphBuilder {
    nodes: Vec<Node>,
    by_name: FxHashMap<String, u32>,
    edges: FxHashMap<(u32, u32), Vec<ImportDetail>>,
}

/// A handle to a node while the graph is being built. Builder handles are
/// not [ModuleId]s, since ids are re-assigned alphabetically by
/// [GraphBuilder::finish].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BuilderId(u32);

impl GraphBuilder {
    /// Create a builder containing only the root directory.
    pub fn new() -> Self {
        let mut builder = Self::default();
        builder.insert(String::new(), NodeKind::Directory, None);
        builder
    }

    fn insert(&mut self, name: String, kind: NodeKind, parent: Option<u32>) -> u32 {
        let id = self.nodes.len() as u32;
        self.by_name.insert(name.clone(), id);
        self.nodes.push(Node {
            name,
            kind,
            parent: parent.map(ModuleId),
            children: Vec::new(),
            scanned: false,
            directives: Vec::new(),
            pre: 0,
            post: 0,
        });
        id
    }

    /// Add a file node by its POSIX path relative to the root, creating its
    /// ancestor directories. Adding an existing file returns its handle.
    pub fn add_file(&mut self, path: &str) -> BuilderId {
        if let Some(id) = self.by_name.get(path) {
            return BuilderId(*id);
        }
        let parent = match path.rsplit_once('/') {
            Some((dir, _)) => self.add_directory(dir),
            None => 0,
        };
        BuilderId(self.insert(path.to_string(), NodeKind::File, Some(parent)))
    }

    fn add_directory(&mut self, path: &str) -> u32 {
        if let Some(id) = self.by_name.get(path) {
            return *id;
        }
        let parent = match path.rsplit_once('/') {
            Some((dir, _)) => self.add_directory(dir),
            None => 0,
        };
        self.insert(path.to_string(), NodeKind::Directory, Some(parent))
    }

    /// Add an external package node, e.g. `npm:react`.
    pub fn add_external(&mut self, name: &str) -> BuilderId {
        self.add_leaf(name, NodeKind::External)
    }

    /// Add a builtin node, e.g. `node:fs`.
    pub fn add_builtin(&mut self, name: &str) -> BuilderId {
        self.add_leaf(name, NodeKind::Builtin)
    }

    fn add_leaf(&mut self, name: &str, kind: NodeKind) -> BuilderId {
        if let Some(id) = self.by_name.get(name) {
            return BuilderId(*id);
        }
        BuilderId(self.insert(name.to_string(), kind, None))
    }

    /// Mark a file as scanned, recording its directives.
    pub fn set_scanned(&mut self, id: BuilderId, directives: Vec<String>) {
        let node = &mut self.nodes[id.0 as usize];
        node.scanned = true;
        node.directives = directives;
    }

    /// Record an import from `importer` to `imported`. Self-imports are
    /// ignored.
    pub fn add_import(&mut self, importer: BuilderId, imported: BuilderId, detail: ImportDetail) {
        if importer == imported {
            return;
        }
        self.edges.entry((importer.0, imported.0)).or_default().push(detail);
    }

    /// Finish building: assign alphabetical ids, sort adjacency lists and
    /// compute the preorder of the module tree.
    pub fn finish(self) -> Graph {
        let GraphBuilder { nodes, edges, .. } = self;

        // Re-number nodes alphabetically. The root ("") sorts first.
        let mut order: Vec<u32> = (0..nodes.len() as u32).collect();
        order.sort_by(|a, b| nodes[*a as usize].name.cmp(&nodes[*b as usize].name));
        let mut remap = vec![0u32; nodes.len()];
        for (new, old) in order.iter().enumerate() {
            remap[*old as usize] = new as u32;
        }

        let mut slots: Vec<Option<Node>> = nodes.into_iter().map(Some).collect();
        let mut nodes: Vec<Node> = order
            .iter()
            .map(|old| {
                let mut node = slots[*old as usize].take().unwrap();
                node.parent = node.parent.map(|parent| ModuleId(remap[parent.0 as usize]));
                node
            })
            .collect();

        for index in 0..nodes.len() {
            if let Some(parent) = nodes[index].parent {
                nodes[parent.index()].children.push(ModuleId(index as u32));
            }
        }
        // Children are pushed in ascending id order, which is alphabetical.

        let by_name = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.name.clone(), ModuleId(index as u32)))
            .collect();

        let mut imports = vec![Vec::new(); nodes.len()];
        let mut importers = vec![Vec::new(); nodes.len()];
        let mut details = FxHashMap::default();
        let mut kinds = FxHashMap::default();
        for ((importer, imported), mut list) in edges {
            let importer = ModuleId(remap[importer as usize]);
            let imported = ModuleId(remap[imported as usize]);
            imports[importer.index()].push(imported);
            importers[imported.index()].push(importer);
            list.sort_by_key(|detail| (detail.start, detail.end));
            list.dedup();
            kinds.insert((importer, imported), list.iter().map(|detail| detail.kind).collect());
            details.insert((importer, imported), list);
        }
        for list in imports.iter_mut().chain(importers.iter_mut()) {
            list.sort_unstable();
        }

        // Preorder of the module tree, then external nodes after it.
        let mut preorder = Vec::with_capacity(nodes.len());
        let mut stack = vec![(ModuleId::ROOT, false)];
        while let Some((id, exiting)) = stack.pop() {
            if exiting {
                nodes[id.index()].post = preorder.len() as u32;
                continue;
            }
            nodes[id.index()].pre = preorder.len() as u32;
            preorder.push(id);
            stack.push((id, true));
            for child in nodes[id.index()].children.iter().rev() {
                stack.push((*child, false));
            }
        }
        for (index, node) in nodes.iter_mut().enumerate() {
            if !node.is_internal() {
                node.pre = preorder.len() as u32;
                preorder.push(ModuleId(index as u32));
                node.post = preorder.len() as u32;
            }
        }

        Graph { nodes, by_name, imports, importers, details, kinds, preorder }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a graph from `(importer, imported)` pairs of file paths. Names
    /// starting with `npm:` or `node:` become external or builtin nodes.
    pub(crate) fn graph_of(imports: &[(&str, &str)]) -> Graph {
        let mut builder = GraphBuilder::new();
        let add = |builder: &mut GraphBuilder, name: &str| {
            if name.starts_with("npm:") {
                builder.add_external(name)
            } else if name.starts_with("node:") {
                builder.add_builtin(name)
            } else {
                builder.add_file(name)
            }
        };
        for (line, (importer, imported)) in imports.iter().enumerate() {
            let importer = add(&mut builder, importer);
            let imported = add(&mut builder, imported);
            let line = line as u32 + 1;
            builder.add_import(
                importer,
                imported,
                ImportDetail {
                    line,
                    column: 1,
                    start: line * 100,
                    end: line * 100 + 10,
                    specifier: String::new(),
                    kind: ImportKind::Value,
                },
            );
        }
        builder.finish()
    }

    pub(crate) fn id(graph: &Graph, name: &str) -> ModuleId {
        graph.id(name).unwrap_or_else(|| panic!("no node `{name}`"))
    }

    #[test]
    fn builds_the_module_tree() {
        let graph = graph_of(&[("a/b/c.ts", "a/d.ts"), ("a/d.ts", "npm:react")]);
        let names: Vec<_> = graph.ids().map(|id| graph.name(id).to_string()).collect();
        assert_eq!(names, vec!["", "a", "a/b", "a/b/c.ts", "a/d.ts", "npm:react"]);

        let a = id(&graph, "a");
        let subtree: Vec<_> = graph.subtree(a).iter().map(|id| graph.name(*id)).collect();
        assert_eq!(subtree, vec!["a", "a/b", "a/b/c.ts", "a/d.ts"]);
        assert!(graph.is_in_subtree(id(&graph, "a/b/c.ts"), a));
        assert!(!graph.is_in_subtree(id(&graph, "npm:react"), a));
        assert_eq!(graph.members(a).len(), 2);
        assert_eq!(graph.node(id(&graph, "npm:react")).parent, None);
        assert_eq!(graph.edge_count(), 2);
    }

    #[test]
    fn subtrees_are_contiguous_despite_sort_order() {
        // `a-b` sorts between `a` and `a/x` by name, but must not be inside
        // the subtree of `a`.
        let graph = graph_of(&[("a/x.ts", "a-b.ts")]);
        let a = id(&graph, "a");
        let subtree: Vec<_> = graph.subtree(a).iter().map(|id| graph.name(*id)).collect();
        assert_eq!(subtree, vec!["a", "a/x.ts"]);
    }
}
