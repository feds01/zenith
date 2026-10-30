//! The graph: the module tree of a project, its external packages and
//! builtins, and the imports between them.

use compact_str::CompactString;
use index_vec::{IndexSlice, IndexVec};
use rayon::prelude::*;
use serde::Serialize;
use thin_vec::ThinVec;
use zenith_utils::location::ByteRange;

use crate::{ImportKind, ImportedNames, NodeSet};

index_vec::define_index_type! {
    /// An opaque handle to a node in a [`Graph`].
    ///
    /// Ids are assigned in alphabetical (byte-wise) order of node names when
    /// the graph is built, so sorting by id is the same as sorting by name.
    /// This keeps every traversal, and therefore every report, deterministic.
    ///
    /// [`ModuleId::index`] runs from 0 up to the graph's node count, so it
    /// can index side tables with one entry per node.
    pub struct ModuleId = u32;
    DEBUG_FORMAT = "ModuleId({})";
    MAX_INDEX = u32::MAX as usize;
}

impl ModuleId {
    /// The root directory of the project, whose name is `""`.
    pub const ROOT: ModuleId = ModuleId::from_raw_unchecked(0);
}

/// What a node in the graph represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    /// A directory inside the project. A directory stands for its whole
    /// subtree and never has imports of its own. An import can still target
    /// it, if it is a package: see [`Graph::is_package`].
    Directory,

    /// A file inside the project.
    File,

    /// An external package, e.g. `npm:react`.
    External,

    /// A runtime builtin, e.g. `node:fs`.
    Builtin,
}

impl NodeKind {
    /// Whether nodes of this kind are part of the project's module tree.
    pub const fn is_internal(self) -> bool {
        matches!(self, NodeKind::Directory | NodeKind::File)
    }
}

/// A node of the module tree, or an external package or builtin.
#[derive(Debug, Clone)]
pub struct Node {
    /// The node's name: a POSIX path relative to the root for internal nodes
    /// (`""` is the root itself), or `<scheme>:<name>` for external packages
    /// and builtins.
    name: Box<str>,

    /// What the node represents.
    kind: NodeKind,

    /// The directive prologue of a scanned file. Almost every node has none,
    /// so this is a [`ThinVec`]: one pointer wide, where a `Vec` is three.
    directives: ThinVec<String>,

    /// The position of the node in [`Graph::preorder`].
    pre: u32,

    /// The end (exclusive) of the node's subtree in [`Graph::preorder`].
    post: u32,
}

// A graph has a node for every file, so a node is kept small: what only some
// graphs need, such as packages, is kept apart from it.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Node>() == 40);

impl Node {
    /// The node's name: a POSIX path relative to the root for internal nodes
    /// (`""` is the root itself), e.g. `web/src/app.ts`, or `<scheme>:<name>`
    /// for external packages and builtins, e.g. `npm:react` and `node:fs`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What the node represents.
    pub fn kind(&self) -> NodeKind {
        self.kind
    }

    /// The directive prologue of a scanned file, e.g. `["use client"]`, as
    /// written. Empty for every other node.
    pub fn directives(&self) -> &[String] {
        &self.directives
    }

    /// Whether this node is part of the project's module tree.
    pub fn is_internal(&self) -> bool {
        self.kind.is_internal()
    }

    /// The last segment of an internal node's name, e.g. `api.ts` for
    /// `web/src/api.ts`. External packages and builtins are not in the tree,
    /// so this is their whole name, e.g. `npm:@acme/ui`.
    pub fn base_name(&self) -> &str {
        match self.name.rsplit_once('/') {
            Some((_, base)) if self.is_internal() => base,
            _ => &self.name,
        }
    }
}

/// One import statement or expression that creates an import edge.
///
/// Details order by their position in the importing file, which is source
/// order. A report that shows a line and column works them out from the
/// range and the file's text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImportDetail {
    /// Where the import is in the importing file.
    pub range: ByteRange,

    /// The specifier as written in the source, e.g. `@acme/ui` or `./api`.
    /// Most specifiers are short enough to be stored inline, without an
    /// allocation of their own.
    pub specifier: CompactString,

    /// How the import is made.
    pub kind: ImportKind,

    /// What it takes from the imported module: `All` unless the names were
    /// recorded.
    pub names: ImportedNames,
}

impl ImportDetail {
    /// This import, as a type import if `type_only`.
    pub(crate) fn with_type_only(&self, type_only: bool) -> Self {
        let mut detail = self.clone();
        if type_only {
            detail.kind = ImportKind::Type;
        }
        detail
    }
}

/// The module tree and import graph of a project.
///
/// The graph holds every internal directory and file, every external package
/// and builtin that is imported, and the imports between them. It is built
/// once by a [`GraphBuilder`](crate::GraphBuilder) and never changes after,
/// so contracts share it and filter it through a
/// [`GraphView`](crate::GraphView) instead.
///
/// Every query returns nodes in ascending id order (that is, by name), except
/// [`Graph::subtree`], which is in tree order. Queries panic if given a
/// [`ModuleId`] that is out of range for this graph.
#[derive(Debug)]
pub struct Graph {
    /// Every node, indexed by id, so sorted by name.
    nodes: IndexVec<ModuleId, Node>,

    /// The children of every node.
    children: Adjacency,

    /// The nodes each node imports. The position of an import in
    /// `imports.targets` is its edge index into `detail_offsets`.
    imports: Adjacency,

    /// The nodes that import each node.
    importers: Adjacency,

    /// Where the details of each edge start in `details`, plus a final entry
    /// for the end, so edge `e` runs up to where edge `e + 1` starts.
    detail_offsets: Vec<usize>,

    /// Every import detail, grouped by edge, in source order within an edge.
    details: Vec<ImportDetail>,

    /// The internal nodes in a preorder traversal of the module tree (with
    /// children in id order), then the external packages and builtins in id
    /// order. Every subtree is a contiguous range of it.
    preorder: Vec<ModuleId>,

    /// The packages, or `None` if there are none, as in most graphs, which
    /// then pay nothing for them.
    packages: Option<Box<Packages>>,

    /// The extensions, without the `.`, that a path glob may leave off a
    /// file's path: those of the languages that the files are in.
    pub(crate) optional_extensions: Vec<&'static str>,

    /// The extensions, without the `.`, of the files that are one module with
    /// the directory beside them of the same name, as Rust's `foo.rs` and
    /// `foo/` are.
    pub(crate) directory_modules: Vec<&'static str>,
}

/// The directories that imports target as a package: a unit of code that is
/// imported as a whole, such as a Go package. They are kept apart from the
/// nodes, so that a graph without any has no room for them.
#[derive(Debug, Clone)]
struct Packages {
    /// The packages.
    flags: NodeSet,

    /// The parent directory of every node, which is `None` for the root, and
    /// for external packages and builtins.
    parents: IndexVec<ModuleId, Option<ModuleId>>,
}

/// A node as the builder hands it to [`Graph::from_parts`].
#[derive(Debug)]
pub(crate) struct NodeParts {
    /// The node's name.
    pub(crate) name: String,

    /// What the node represents.
    pub(crate) kind: NodeKind,

    /// The directive prologue of a scanned file.
    pub(crate) directives: ThinVec<String>,

    /// Whether the node is a directory that imports target as a package.
    pub(crate) package: bool,

    /// The parent directory, or `None` for the root and for external
    /// packages and builtins.
    pub(crate) parent: Option<ModuleId>,
}

impl Graph {
    /// Assemble a graph.
    ///
    /// `nodes` must be in id order: sorted by name, with unique names, so the
    /// root (`""`) comes first. Every internal node other than the root must
    /// have a parent directory. `imports` must be sorted, without duplicates.
    pub(crate) fn from_parts(
        nodes: Vec<NodeParts>,
        imports: Vec<(ModuleId, ModuleId, ImportDetail)>,
        optional_extensions: Vec<&'static str>,
        directory_modules: Vec<&'static str>,
    ) -> Self {
        debug_assert!(imports.is_sorted(), "imports must be sorted for the edge lookup");
        let node_count = nodes.len();

        // Children are grouped in ascending id order, so each list is sorted.
        let children = Adjacency::group(
            node_count,
            nodes
                .iter()
                .enumerate()
                .filter_map(|(index, node)| Some((node.parent?, ModuleId::new(index)))),
        );

        let ImportParts { imports, importers, detail_offsets, details } =
            ImportParts::group(node_count, imports);

        let flags: NodeSet = (nodes.iter().enumerate())
            .filter(|(_, node)| node.package)
            .map(|(index, _)| ModuleId::new(index))
            .collect();
        let packages = (!flags.is_empty()).then(|| {
            let parents = nodes.iter().map(|node| node.parent).collect();
            Box::new(Packages { flags, parents })
        });

        let mut nodes: IndexVec<ModuleId, Node> = nodes
            .into_iter()
            .map(|node| Node {
                name: node.name.into_boxed_str(),
                kind: node.kind,
                directives: node.directives,
                pre: 0,
                post: 0,
            })
            .collect();
        let preorder = assign_preorder(&mut nodes, &children);

        Self {
            nodes,
            children,
            imports,
            importers,
            detail_offsets,
            details,
            preorder,
            packages,
            optional_extensions,
            directory_modules,
        }
    }

    /// This graph's module tree, with `imports` instead of its own: each an
    /// `(importer, imported, detail)` between its nodes, in any order.
    pub(crate) fn with_imports(
        &self,
        mut imports: Vec<(ModuleId, ModuleId, ImportDetail)>,
    ) -> Self {
        imports.par_sort_unstable();
        imports.dedup();
        let ImportParts { imports, importers, detail_offsets, details } =
            ImportParts::group(self.node_count(), imports);
        Self {
            nodes: self.nodes.clone(),
            children: self.children.clone(),
            imports,
            importers,
            detail_offsets,
            details,
            preorder: self.preorder.clone(),
            packages: self.packages.clone(),
            optional_extensions: self.optional_extensions.clone(),
            directory_modules: self.directory_modules.clone(),
        }
    }

    /// The number of nodes in the graph. There is always at least one, the
    /// root.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Get a node.
    pub fn node(&self, id: ModuleId) -> &Node {
        &self.nodes[id]
    }

    /// The name of a node.
    pub fn name(&self, id: ModuleId) -> &str {
        self.node(id).name()
    }

    /// Look up a node by name.
    pub fn id(&self, name: &str) -> Option<ModuleId> {
        self.nodes.binary_search_by(|node| node.name().cmp(name)).ok()
    }

    /// Every node id, in ascending order.
    pub fn ids(&self) -> impl ExactSizeIterator<Item = ModuleId> + use<> {
        self.nodes.indices()
    }

    /// The ids of the nodes whose name starts with `prefix`, in ascending
    /// order. Ids are in name order, so these are a contiguous range.
    pub(crate) fn ids_with_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = ModuleId> + 'a {
        let start = self.nodes.raw.partition_point(|node| node.name() < prefix);
        (start..self.node_count())
            .map(ModuleId::new)
            .take_while(move |id| self.name(*id).starts_with(prefix))
    }

    /// The children of a node. Only directories have children.
    pub fn children(&self, id: ModuleId) -> &[ModuleId] {
        self.children.get(id)
    }

    /// The node itself followed by all of its descendants, in a preorder
    /// traversal of the module tree. For a file, an external package or a
    /// builtin, this is just the node.
    pub fn subtree(&self, id: ModuleId) -> &[ModuleId] {
        let node = self.node(id);
        &self.preorder[node.pre as usize..node.post as usize]
    }

    /// Whether `id` is `ancestor` or one of its descendants.
    pub fn is_in_subtree(&self, id: ModuleId, ancestor: ModuleId) -> bool {
        let node = self.node(id);
        let ancestor = self.node(ancestor);
        ancestor.pre <= node.pre && node.pre < ancestor.post
    }

    /// The nodes that can take part in imports in the subtree of `id`: every
    /// file in it and every package, that is, every directory that imports
    /// target, or the node itself for an external package or builtin.
    ///
    /// A package is a member of the same units as the files in it. It is
    /// imported but never imports, and it is not a file: count files with
    /// [`Self::file_count`], not by the number of members.
    pub fn members(&self, id: ModuleId) -> NodeSet {
        let members = self.subtree(id).iter().copied();
        match &self.packages {
            None => {
                members.filter(|member| self.node(*member).kind != NodeKind::Directory).collect()
            }
            Some(packages) => members
                .filter(|member| {
                    self.node(*member).kind != NodeKind::Directory
                        || packages.flags.contains(*member)
                })
                .collect(),
        }
    }

    /// The number of files in the subtree of `id`, or 1 for an external
    /// package or builtin, which stands for itself. Packages are directories,
    /// so they are not counted.
    pub fn file_count(&self, id: ModuleId) -> usize {
        self.subtree(id).iter().filter(|node| self.node(**node).kind != NodeKind::Directory).count()
    }

    /// Whether any directory of the graph is a package.
    pub fn has_packages(&self) -> bool {
        self.packages.is_some()
    }

    /// Whether imports target the directory `id` as a package, the way a Go
    /// import names a directory rather than a file.
    ///
    /// An import of a package is an import of every file directly in the
    /// directory, but it is stored once, as an import of the directory
    /// itself, from the file that makes it. A package never imports anything:
    /// what it reaches is what its files reach, which the searches for chains
    /// follow without the graph storing an import for it.
    pub fn is_package(&self, id: ModuleId) -> bool {
        self.packages.as_ref().is_some_and(|packages| packages.flags.contains(id))
    }

    /// The files directly in the package `id`, in ascending id order. There
    /// are none if `id` is not a package. Its subdirectories are not part of
    /// it.
    pub fn package_files(&self, id: ModuleId) -> impl Iterator<Item = ModuleId> + '_ {
        let files = if self.is_package(id) { self.children(id) } else { &[] };
        files.iter().copied().filter(|child| self.node(*child).kind == NodeKind::File)
    }

    /// The package that the file `id` is directly in, if its directory is
    /// one.
    pub fn package_of(&self, id: ModuleId) -> Option<ModuleId> {
        let packages = self.packages.as_ref()?;
        if self.node(id).kind != NodeKind::File {
            return None;
        }
        let parent = packages.parents[id]?;
        packages.flags.contains(parent).then_some(parent)
    }

    /// Whether `file` is one of the files directly in the package `package`.
    ///
    /// A hop from a package to one of its files is not an import. It is how a
    /// chain of imports goes on from a package to what its files import, and
    /// no contract reports it.
    pub fn is_package_member(&self, package: ModuleId, file: ModuleId) -> bool {
        self.package_of(file) == Some(package)
    }

    /// The nodes that `id` imports directly.
    pub fn imports_of(&self, id: ModuleId) -> &[ModuleId] {
        self.imports.get(id)
    }

    /// The nodes that import `id` directly.
    pub fn importers_of(&self, id: ModuleId) -> &[ModuleId] {
        self.importers.get(id)
    }

    /// Whether `importer` imports `imported` directly.
    pub fn has_import(&self, importer: ModuleId, imported: ModuleId) -> bool {
        self.imports_of(importer).binary_search(&imported).is_ok()
    }

    /// The details of every import from `importer` to `imported`, in source
    /// order. Empty if `importer` does not import `imported`.
    pub fn details(&self, importer: ModuleId, imported: ModuleId) -> &[ImportDetail] {
        let Ok(position) = self.imports_of(importer).binary_search(&imported) else {
            return &[];
        };
        let edge = self.imports.start(importer) + position;
        &self.details[self.detail_offsets[edge]..self.detail_offsets[edge + 1]]
    }

    /// Every `(importer, imported)` pair, in ascending order.
    pub fn edges(&self) -> impl Iterator<Item = (ModuleId, ModuleId)> + '_ {
        self.ids().flat_map(move |importer| {
            self.imports_of(importer).iter().map(move |imported| (importer, *imported))
        })
    }

    /// The number of import statements and expressions: the details of
    /// every import, rather than the number of `(importer, imported)` pairs.
    pub fn detail_count(&self) -> usize {
        self.details.len()
    }
}

/// Record the preorder position and subtree end of every node, and return
/// the preorder: the module tree depth first from the root, then the external
/// packages and builtins.
fn assign_preorder(
    nodes: &mut IndexSlice<ModuleId, [Node]>,
    children: &Adjacency,
) -> Vec<ModuleId> {
    /// A step of the depth-first traversal.
    enum Visit {
        /// Record the node and queue its children.
        Enter(ModuleId),
        /// Every descendant of the node has been recorded.
        Exit(ModuleId),
    }

    /// The position of the next node in the preorder, or the end of the
    /// preorder. It fits in a `u32`, because the builder keeps the node count
    /// below `u32::MAX`.
    fn position(preorder: &[ModuleId]) -> u32 {
        preorder.len() as u32
    }

    let mut preorder = Vec::with_capacity(nodes.len());
    let mut stack = vec![Visit::Enter(ModuleId::ROOT)];
    while let Some(visit) = stack.pop() {
        match visit {
            Visit::Enter(id) => {
                nodes[id].pre = position(&preorder);
                preorder.push(id);
                stack.push(Visit::Exit(id));
                stack.extend(children.get(id).iter().rev().map(|child| Visit::Enter(*child)));
            }
            Visit::Exit(id) => nodes[id].post = position(&preorder),
        }
    }

    for (id, node) in nodes.iter_mut_enumerated() {
        if !node.is_internal() {
            node.pre = position(&preorder);
            preorder.push(id);
            node.post = position(&preorder);
        }
    }
    preorder
}

/// The imports of a graph, grouped for lookups both ways.
struct ImportParts {
    /// The nodes each node imports.
    imports: Adjacency,

    /// The nodes that import each node.
    importers: Adjacency,

    /// Where the details of each edge start in `details`, and where the last
    /// one ends.
    detail_offsets: Vec<usize>,

    /// Every detail, grouped by edge.
    details: Vec<ImportDetail>,
}

impl ImportParts {
    /// Group `imports` between `node_count` nodes, sorted by importer, then
    /// imported, then detail, so that each run of one pair is an edge, and
    /// the edges are sorted too.
    fn group(node_count: usize, imports: Vec<(ModuleId, ModuleId, ImportDetail)>) -> Self {
        let mut edges = Vec::new();
        let mut detail_offsets = Vec::new();
        let mut details = Vec::with_capacity(imports.len());
        for (importer, imported, detail) in imports {
            if edges.last() != Some(&(importer, imported)) {
                edges.push((importer, imported));
                detail_offsets.push(details.len());
            }
            details.push(detail);
        }
        detail_offsets.push(details.len());

        let imports = Adjacency::group(node_count, edges.iter().copied());
        let importers = Adjacency::group(
            node_count,
            edges.iter().map(|(importer, imported)| (*imported, *importer)),
        );
        Self { imports, importers, detail_offsets, details }
    }
}

/// The adjacency lists of every node, stored back to back (compressed sparse
/// rows), so a relation over any number of nodes takes two allocations.
#[derive(Debug, Clone)]
struct Adjacency {
    /// Node `i`'s list is `targets[offsets[i]..offsets[i + 1]]`.
    offsets: Vec<usize>,

    /// Every list, in node order.
    targets: Vec<ModuleId>,
}

impl Adjacency {
    /// Group `(node, target)` pairs into each node's list. Lists keep the
    /// order of the pairs (this is a stable counting sort), so pairs in
    /// ascending target order make sorted lists.
    fn group(node_count: usize, pairs: impl Iterator<Item = (ModuleId, ModuleId)> + Clone) -> Self {
        // Count each node's targets, then turn the counts into start offsets.
        let mut offsets = vec![0; node_count + 1];
        for (node, _) in pairs.clone() {
            offsets[node.index()] += 1;
        }
        let mut total = 0;
        for offset in &mut offsets {
            let count = *offset;
            *offset = total;
            total += count;
        }

        let mut next = offsets.clone();
        let mut targets = vec![ModuleId::ROOT; total];
        for (node, target) in pairs {
            let slot = &mut next[node.index()];
            targets[*slot] = target;
            *slot += 1;
        }
        Self { offsets, targets }
    }

    /// The list of a node.
    fn get(&self, id: ModuleId) -> &[ModuleId] {
        &self.targets[self.start(id)..self.offsets[id.index() + 1]]
    }

    /// Where the list of a node starts in `targets`.
    fn start(&self, id: ModuleId) -> usize {
        self.offsets[id.index()]
    }
}

#[cfg(test)]
mod tests {
    use thin_vec::thin_vec;

    use super::*;
    use crate::{
        GraphBuilder,
        testing::{detail, graph_of, id, names},
    };

    #[test]
    fn ids_follow_the_alphabetical_order_of_names() {
        let graph =
            graph_of(&[("z.ts", "node:fs"), ("a/b/c.ts", "a/d.ts"), ("a/d.ts", "npm:react")]);
        assert_eq!(
            names(&graph, graph.ids()),
            ["", "a", "a/b", "a/b/c.ts", "a/d.ts", "node:fs", "npm:react", "z.ts"]
        );
        assert_eq!(graph.node_count(), 8);
        assert_eq!(graph.name(ModuleId::ROOT), "");
        assert_eq!(graph.id(""), Some(ModuleId::ROOT));
        assert_eq!(graph.id("a/d.ts").map(|id| graph.name(id)), Some("a/d.ts"));
        assert_eq!(graph.id("a/c.ts"), None);
        assert_eq!(graph.id("zz.ts"), None);
    }

    #[test]
    fn nodes_know_their_kind_and_base_name() {
        let graph = graph_of(&[("web/src/app.ts", "npm:@acme/ui"), ("web/src/app.ts", "node:fs")]);
        let node = |name| graph.node(id(&graph, name));

        assert_eq!(node("").kind(), NodeKind::Directory);
        assert_eq!(node("web/src").kind(), NodeKind::Directory);
        assert_eq!(node("web/src/app.ts").kind(), NodeKind::File);
        assert_eq!(node("npm:@acme/ui").kind(), NodeKind::External);
        assert_eq!(node("node:fs").kind(), NodeKind::Builtin);
        assert!(node("web/src").is_internal() && node("web/src/app.ts").is_internal());
        assert!(!node("npm:@acme/ui").is_internal() && !node("node:fs").is_internal());

        assert_eq!(node("").base_name(), "");
        assert_eq!(node("web").base_name(), "web");
        assert_eq!(node("web/src/app.ts").base_name(), "app.ts");
        assert_eq!(node("npm:@acme/ui").base_name(), "npm:@acme/ui");
        assert_eq!(node("node:fs").base_name(), "node:fs");
    }

    #[test]
    fn children_are_sorted_by_name() {
        let graph = graph_of(&[("b.ts", "a/y.ts"), ("a/x.ts", "npm:react"), ("a/y.ts", "a/x.ts")]);
        assert_eq!(names(&graph, graph.children(ModuleId::ROOT).iter().copied()), ["a", "b.ts"]);
        assert_eq!(
            names(&graph, graph.children(id(&graph, "a")).iter().copied()),
            ["a/x.ts", "a/y.ts"]
        );
        assert!(graph.children(id(&graph, "b.ts")).is_empty());
        assert!(graph.children(id(&graph, "npm:react")).is_empty());
    }

    #[test]
    fn subtrees_are_contiguous_despite_sort_order() {
        // `a-b.ts` sorts between `a` and `a/x.ts` by name, but is not inside
        // the subtree of `a`.
        let graph = graph_of(&[("a/x.ts", "a-b.ts"), ("a/x.ts", "npm:react")]);
        let a = id(&graph, "a");
        assert_eq!(names(&graph, graph.ids()), ["", "a", "a-b.ts", "a/x.ts", "npm:react"]);
        assert_eq!(names(&graph, graph.subtree(a).iter().copied()), ["a", "a/x.ts"]);
        assert_eq!(
            names(&graph, graph.subtree(ModuleId::ROOT).iter().copied()),
            ["", "a", "a/x.ts", "a-b.ts"]
        );

        let react = id(&graph, "npm:react");
        assert_eq!(graph.subtree(react), [react]);
        let file = id(&graph, "a-b.ts");
        assert_eq!(graph.subtree(file), [file]);
    }

    #[test]
    fn subtree_membership_follows_the_module_tree() {
        let graph = graph_of(&[("a/b/c.ts", "a-b.ts"), ("a/b/c.ts", "npm:react")]);
        let [root, a, b, c, sibling, react] =
            ["", "a", "a/b", "a/b/c.ts", "a-b.ts", "npm:react"].map(|name| id(&graph, name));

        assert!(graph.is_in_subtree(c, a));
        assert!(graph.is_in_subtree(c, b));
        assert!(graph.is_in_subtree(a, a));
        assert!(graph.is_in_subtree(sibling, root));
        assert!(!graph.is_in_subtree(sibling, a));
        assert!(!graph.is_in_subtree(a, c));
        assert!(!graph.is_in_subtree(react, root));
        assert!(!graph.is_in_subtree(c, react));
        assert!(graph.is_in_subtree(react, react));
    }

    #[test]
    fn members_are_the_files_externals_and_builtins_of_a_subtree() {
        let graph =
            graph_of(&[("a/b/c.ts", "a/d.ts"), ("a/d.ts", "npm:react"), ("e.ts", "node:fs")]);
        let members = |name| names(&graph, graph.members(id(&graph, name)).iter());

        assert_eq!(members("a"), ["a/b/c.ts", "a/d.ts"]);
        assert_eq!(members(""), ["a/b/c.ts", "a/d.ts", "e.ts"]);
        assert_eq!(members("e.ts"), ["e.ts"]);
        assert_eq!(members("npm:react"), ["npm:react"]);
        assert_eq!(members("node:fs"), ["node:fs"]);
    }

    #[test]
    fn imports_and_importers_are_sorted_in_both_directions() {
        let graph = graph_of(&[
            ("c.ts", "npm:react"),
            ("c.ts", "a.ts"),
            ("b.ts", "a.ts"),
            ("c.ts", "b.ts"),
            ("b.ts", "npm:react"),
        ]);
        let [a, b, c, react] = ["a.ts", "b.ts", "c.ts", "npm:react"].map(|name| id(&graph, name));

        assert_eq!(graph.imports_of(c), [a, b, react]);
        assert_eq!(graph.imports_of(b), [a, react]);
        assert!(graph.imports_of(a).is_empty());
        assert!(graph.imports_of(react).is_empty());
        assert_eq!(graph.importers_of(a), [b, c]);
        assert_eq!(graph.importers_of(react), [b, c]);
        assert!(graph.importers_of(c).is_empty());

        assert!(graph.has_import(c, a));
        assert!(!graph.has_import(a, c));
        assert!(!graph.has_import(ModuleId::ROOT, a));
        assert_eq!(
            graph.edges().collect::<Vec<_>>(),
            [(b, a), (b, react), (c, a), (c, b), (c, react)]
        );
    }

    #[test]
    fn details_are_kept_per_edge_in_source_order() {
        let mut builder = GraphBuilder::new();
        let app = builder.add_file("web/src/app.ts").unwrap();
        let api = builder.add_file("web/src/api.ts").unwrap();
        let react = builder.add_external("npm:react").unwrap();
        builder.add_import(app, api, detail(9, ImportKind::Dynamic));
        builder.add_import(app, react, detail(1, ImportKind::Value));
        builder.add_import(app, api, detail(2, ImportKind::Type));
        let graph = builder.finish();
        let [app, api, react] =
            ["web/src/app.ts", "web/src/api.ts", "npm:react"].map(|name| id(&graph, name));

        assert_eq!(
            graph.details(app, api),
            [detail(2, ImportKind::Type), detail(9, ImportKind::Dynamic)]
        );
        assert_eq!(graph.details(app, react), [detail(1, ImportKind::Value)]);
        assert!(graph.details(api, app).is_empty());
        assert!(graph.details(react, app).is_empty());
        assert_eq!(graph.detail_count(), 3);
    }

    #[test]
    fn details_belong_to_their_own_edge_across_importers() {
        // Every importer has imports, so each one's edges start after the
        // edges of the importers before it.
        let mut builder = GraphBuilder::new();
        let [a, b, c] = ["a.ts", "b.ts", "c.ts"].map(|path| builder.add_file(path).unwrap());
        builder.add_import(a, b, detail(1, ImportKind::Value));
        builder.add_import(a, b, detail(2, ImportKind::Type));
        builder.add_import(b, c, detail(3, ImportKind::Require));
        builder.add_import(c, a, detail(4, ImportKind::Dynamic));
        builder.add_import(c, b, detail(5, ImportKind::Value));
        builder.add_import(c, b, detail(6, ImportKind::Value));
        let graph = builder.finish();
        let [a, b, c] = ["a.ts", "b.ts", "c.ts"].map(|name| id(&graph, name));

        assert_eq!(
            graph.details(a, b),
            [detail(1, ImportKind::Value), detail(2, ImportKind::Type)]
        );
        assert_eq!(graph.details(b, c), [detail(3, ImportKind::Require)]);
        assert_eq!(graph.details(c, a), [detail(4, ImportKind::Dynamic)]);
        assert_eq!(
            graph.details(c, b),
            [detail(5, ImportKind::Value), detail(6, ImportKind::Value)]
        );
        assert!(graph.details(a, c).is_empty());
        assert!(graph.details(b, a).is_empty());
        assert_eq!(graph.detail_count(), 6);
        assert_eq!(graph.edges().count(), 4);
    }

    #[test]
    fn scanned_files_keep_their_directives() {
        let mut builder = GraphBuilder::new();
        let page = builder.add_file("web/app/page.tsx").unwrap();
        builder.set_directives(page, thin_vec!["use strict".to_string()]);
        builder.set_directives(page, thin_vec!["use client".to_string(), "use strict".to_string()]);
        builder.add_file("web/app/styles.css").unwrap();
        let graph = builder.finish();

        assert_eq!(
            graph.node(id(&graph, "web/app/page.tsx")).directives(),
            ["use client", "use strict"]
        );
        assert!(graph.node(id(&graph, "web/app/styles.css")).directives().is_empty());
        assert!(graph.node(id(&graph, "web/app")).directives().is_empty());
    }

    /// `cmd` imports the packages `internal/db` and `internal/log`, and the
    /// package `internal/db` has a subdirectory.
    fn packages() -> Graph {
        graph_of(&[
            ("cmd/main.go", "internal/db/"),
            ("cmd/main.go", "internal/log/"),
            ("internal/db/conn.go", "internal/log/"),
            ("internal/db/query.go", "node:fmt"),
            ("internal/db/sub/deep.go", "node:os"),
        ])
    }

    #[test]
    fn a_directory_that_is_imported_is_a_package() {
        let graph = packages();
        let package = |name| graph.is_package(id(&graph, name));

        assert!(package("internal/db") && package("internal/log"));
        assert!(!package("internal") && !package("internal/db/sub") && !package("cmd"));
        assert!(!package("") && !package("cmd/main.go") && !package("node:fmt"));
        assert_eq!(graph.node(id(&graph, "internal/db")).kind(), NodeKind::Directory);
        assert!(graph.has_packages());
        assert!(!graph_of(&[("a.ts", "b.ts")]).has_packages());
    }

    #[test]
    fn an_import_of_a_package_is_an_edge_to_its_directory() {
        let graph = packages();
        let [main, db, log, conn] =
            ["cmd/main.go", "internal/db", "internal/log", "internal/db/conn.go"]
                .map(|name| id(&graph, name));

        assert_eq!(graph.imports_of(main), [db, log]);
        assert_eq!(graph.importers_of(db), [main]);
        assert_eq!(graph.importers_of(log), [main, conn]);
        assert!(graph.has_import(main, db));
        assert_eq!(graph.details(main, db), [detail(1, ImportKind::Value)]);
        assert_eq!(graph.detail_count(), 5);
    }

    #[test]
    fn nothing_links_a_package_to_its_files_as_an_import() {
        let graph = packages();
        let [db, conn, query, deep] = [
            "internal/db",
            "internal/db/conn.go",
            "internal/db/query.go",
            "internal/db/sub/deep.go",
        ]
        .map(|name| id(&graph, name));

        // A package imports nothing and is imported by nothing in it, so no
        // edge, detail or import statistic stands for the link.
        assert!(graph.imports_of(db).is_empty());
        for file in [conn, query, deep] {
            assert!(!graph.importers_of(file).contains(&db));
            assert!(graph.details(db, file).is_empty() && graph.details(file, db).is_empty());
        }
        assert_eq!(graph.edges().count(), 5);
        assert_eq!(graph.detail_count(), 5);
        assert!(graph.edges().all(|(importer, _)| graph.node(importer).kind() == NodeKind::File));
    }

    #[test]
    fn members_include_packages_but_a_package_is_not_a_file() {
        let graph = packages();
        let members = |name| names(&graph, graph.members(id(&graph, name)).iter());

        assert_eq!(
            members("internal"),
            [
                "internal/db",
                "internal/db/conn.go",
                "internal/db/query.go",
                "internal/db/sub/deep.go",
                "internal/log",
            ]
        );
        assert_eq!(members("internal/db/sub"), ["internal/db/sub/deep.go"]);
        assert_eq!(members("cmd"), ["cmd/main.go"]);
        assert_eq!(members("internal/log"), ["internal/log"]);

        let files = |name| {
            let members = graph.members(id(&graph, name));
            members.iter().filter(|member| graph.node(*member).kind() == NodeKind::File).count()
        };
        assert_eq!(files("internal"), 3);
    }

    #[test]
    fn a_package_holds_the_files_directly_in_it() {
        let graph = packages();
        let [db, sub, main] =
            ["internal/db", "internal/db/sub", "cmd/main.go"].map(|name| id(&graph, name));
        let files = |package| names(&graph, graph.package_files(package));

        // Not the subdirectory, nor what is in it.
        assert_eq!(files(db), ["internal/db/conn.go", "internal/db/query.go"]);
        assert!(files(sub).is_empty());
        assert!(files(main).is_empty());
        assert!(files(ModuleId::ROOT).is_empty());

        let conn = id(&graph, "internal/db/conn.go");
        let deep = id(&graph, "internal/db/sub/deep.go");
        assert_eq!(graph.package_of(conn), Some(db));
        assert_eq!(graph.package_of(deep), None);
        assert_eq!(graph.package_of(main), None);
        assert_eq!(graph.package_of(db), None);
        assert!(graph.is_package_member(db, conn));
        assert!(!graph.is_package_member(db, deep));
        assert!(!graph.is_package_member(conn, db));
    }

    #[test]
    fn the_root_can_be_a_package() {
        let mut builder = GraphBuilder::new();
        let tool = builder.add_file("tools/gen/main.go").unwrap();
        builder.add_file("lib.go").unwrap();
        builder.add_package_import(tool, "", detail(1, ImportKind::Value)).unwrap();
        let graph = builder.finish();

        let [tool, lib] = ["tools/gen/main.go", "lib.go"].map(|name| id(&graph, name));
        assert!(graph.is_package(ModuleId::ROOT));
        assert_eq!(graph.imports_of(tool), [ModuleId::ROOT]);
        assert_eq!(graph.package_of(lib), Some(ModuleId::ROOT));
        assert_eq!(graph.package_of(tool), None);
        assert_eq!(
            names(&graph, graph.members(ModuleId::ROOT).iter()),
            ["", "lib.go", "tools/gen/main.go"]
        );
    }

    #[test]
    fn a_graph_with_other_imports_has_the_same_packages() {
        let graph = packages();
        let rebuilt = graph.with_imports(Vec::new());
        assert!(rebuilt.has_packages());
        assert!(rebuilt.is_package(id(&rebuilt, "internal/db")));
        assert_eq!(
            rebuilt.package_of(id(&rebuilt, "internal/db/conn.go")),
            Some(id(&rebuilt, "internal/db"))
        );
        assert_eq!(rebuilt.detail_count(), 0);
    }
}
