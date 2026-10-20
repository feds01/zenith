//! Building a [`Graph`] incrementally, as files are parsed and their imports
//! resolved.

use std::{fmt, mem};

use index_vec::{IndexVec, index_vec};
use rustc_hash::FxHashMap;
use thin_vec::ThinVec;
use zenith_utils::path::{is_relative_posix, posix_parent};

use crate::{Graph, ImportDetail, ModuleId, NodeKind, graph::NodeParts};

index_vec::define_index_type! {
    /// A handle to a file while a graph is being built.
    ///
    /// Only files have imports, so [`GraphBuilder::add_import`] takes the
    /// importer as a file handle. Handles are not [`ModuleId`]s, since ids
    /// are assigned by [`GraphBuilder::finish`], and a handle only means
    /// something to the builder that returned it.
    pub struct FileHandle = u32;
    DEBUG_FORMAT = "FileHandle({})";
    MAX_INDEX = u32::MAX as usize;
}

index_vec::define_index_type! {
    /// A handle to anything an import can point at while a graph is being
    /// built: a file, an external package, a builtin or a package directory.
    /// Files convert into it.
    pub struct NodeHandle = u32;
    DEBUG_FORMAT = "NodeHandle({})";
    MAX_INDEX = u32::MAX as usize;
}

impl From<FileHandle> for NodeHandle {
    fn from(file: FileHandle) -> Self {
        NodeHandle::from_foreign(file)
    }
}

/// Why a node could not be added to a graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// A file path is not a normalised POSIX path relative to the root: it is
    /// empty, starts or ends with `/`, or has an empty, `.` or `..` segment.
    InvalidPath {
        /// The path as given.
        path: String,
    },

    /// An external package or builtin name is not of the form
    /// `<scheme>:<name>`, e.g. `npm:react`.
    InvalidName {
        /// The name as given.
        name: String,
    },

    /// A name already belongs to a node of another kind, e.g. a file path
    /// that is also the directory of another file.
    KindConflict {
        /// The name of the node.
        name: String,

        /// The kind the node already has.
        existing: NodeKind,

        /// The kind it was added as.
        added: NodeKind,
    },

    /// The graph already has `u32::MAX` nodes, the most it can hold.
    TooManyNodes,
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath { path } => {
                write!(f, "`{path}` is not a normalised POSIX path relative to the project root")
            }
            Self::InvalidName { name } => {
                write!(f, "`{name}` is not a name of the form `<scheme>:<name>`")
            }
            Self::KindConflict { name, existing, added } => write!(
                f,
                "`{name}` is already {}, so it cannot also be {}",
                describe(*existing),
                describe(*added)
            ),
            Self::TooManyNodes => write!(f, "the graph cannot hold more than {} nodes", u32::MAX),
        }
    }
}

impl std::error::Error for GraphError {}

/// A node kind for messages, with its article.
fn describe(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Directory => "a directory",
        NodeKind::File => "a file",
        NodeKind::External => "an external package",
        NodeKind::Builtin => "a builtin",
    }
}

/// A node while the graph is being built. Its name is the key of
/// [`GraphBuilder::handles`].
#[derive(Debug)]
struct PendingNode {
    /// What the node represents.
    kind: NodeKind,

    /// Whether imports target the node as a package, which only a directory
    /// can be.
    package: bool,

    /// The handle of the parent directory, or `None` for the root and for
    /// external packages and builtins.
    parent: Option<NodeHandle>,

    /// The directive prologue of a scanned file.
    directives: ThinVec<String>,
}

/// An import while the graph is being built.
#[derive(Debug)]
struct PendingImport {
    /// The handle of the importing file.
    importer: NodeHandle,

    /// The handle of the imported node.
    imported: NodeHandle,

    /// The import statement or expression.
    detail: ImportDetail,
}

/// Incrementally builds a [`Graph`].
///
/// Nodes and imports can be added in any order, and adding a node that exists
/// returns its handle. [`GraphBuilder::finish`] sorts everything, so the
/// graph does not depend on the order of insertion.
#[derive(Debug)]
pub struct GraphBuilder {
    /// The handle of every node, by name. This holds the only copy of each
    /// name until [`GraphBuilder::finish`] moves the names into the graph.
    handles: FxHashMap<String, NodeHandle>,

    /// Every node, indexed by handle. The root directory is handle 0.
    nodes: IndexVec<NodeHandle, PendingNode>,

    /// Every import, in insertion order.
    imports: Vec<PendingImport>,
}

impl Default for GraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphBuilder {
    /// Create a builder containing only the root directory.
    pub fn new() -> Self {
        let root = PendingNode {
            kind: NodeKind::Directory,
            package: false,
            parent: None,
            directives: ThinVec::new(),
        };
        let nodes = index_vec![root];
        let handles = FxHashMap::from_iter([(String::new(), NodeHandle::new(0))]);
        Self { handles, nodes, imports: Vec::new() }
    }

    /// Add a file by its POSIX path relative to the root, e.g.
    /// `web/src/app.ts`, along with its ancestor directories. Adding a file
    /// that exists returns its handle.
    pub fn add_file(&mut self, path: &str) -> Result<FileHandle, GraphError> {
        if !is_relative_posix(path) {
            return Err(GraphError::InvalidPath { path: path.to_string() });
        }
        if let Some(&handle) = self.handles.get(path) {
            return self.expect_kind(handle, path, NodeKind::File).map(FileHandle::from_foreign);
        }
        let parent = self.directory(posix_parent(path))?;
        self.insert(path, NodeKind::File, Some(parent)).map(FileHandle::from_foreign)
    }

    /// Add an external package by its name, e.g. `npm:react`. Adding one that
    /// exists returns its handle.
    pub fn add_external(&mut self, name: &str) -> Result<NodeHandle, GraphError> {
        self.add_leaf(name, NodeKind::External)
    }

    /// Add a builtin by its name, e.g. `node:fs`. Adding one that exists
    /// returns its handle.
    pub fn add_builtin(&mut self, name: &str) -> Result<NodeHandle, GraphError> {
        self.add_leaf(name, NodeKind::Builtin)
    }

    /// Record the directive prologue of a scanned file, e.g.
    /// `["use client"]`, replacing any recorded before.
    pub fn set_directives(&mut self, file: FileHandle, directives: ThinVec<String>) {
        self.nodes[NodeHandle::from(file)].directives = directives;
    }

    /// Record an import from `importer` to `imported`.
    ///
    /// A file that imports itself gets no edge, and a detail recorded twice
    /// is kept once.
    pub fn add_import(
        &mut self,
        importer: FileHandle,
        imported: impl Into<NodeHandle>,
        detail: ImportDetail,
    ) {
        let (importer, imported) = (importer.into(), imported.into());
        if importer != imported {
            self.imports.push(PendingImport { importer, imported, detail });
        }
    }

    /// Record an import from `importer` of the package in the directory
    /// `package`, a POSIX path relative to the root such as `internal/db`, or
    /// `""` for the root. The directory is added, with its ancestors, if it
    /// is not there, and from now on is a package, whose name is its path.
    ///
    /// The import is of the whole package, so it makes one edge, to the
    /// directory, however many files the package has. It is the one place
    /// where an import targets a directory.
    ///
    /// A file that imports the package it is in gets no edge, as one that
    /// imports itself gets none, and a detail recorded twice is kept once.
    /// Nothing is added unless `package` is a directory path, or the root.
    pub fn add_package_import(
        &mut self,
        importer: FileHandle,
        package: &str,
        detail: ImportDetail,
    ) -> Result<(), GraphError> {
        let package = match package.is_empty() {
            true => NodeHandle::new(0),
            false if is_relative_posix(package) => self.directory(package)?,
            false => return Err(GraphError::InvalidPath { path: package.to_string() }),
        };
        let importer = NodeHandle::from(importer);
        if self.nodes[importer].parent != Some(package) {
            self.nodes[package].package = true;
            self.imports.push(PendingImport { importer, imported: package, detail });
        }
        Ok(())
    }

    /// Finish building: assign ids in name order, and sort every list.
    pub fn finish(self) -> Graph {
        let GraphBuilder { handles, mut nodes, imports } = self;

        let mut names: IndexVec<NodeHandle, String> = index_vec![String::new(); nodes.len()];
        for (name, handle) in handles {
            names[handle] = name;
        }

        // Names are unique, so sorting by them is a total order, and the root
        // (`""`) comes first.
        let mut order: Vec<NodeHandle> = nodes.indices().collect();
        order.sort_unstable_by(|a, b| names[*a].cmp(&names[*b]));
        let mut ids: IndexVec<NodeHandle, ModuleId> = index_vec![ModuleId::ROOT; nodes.len()];
        for (index, handle) in order.iter().enumerate() {
            ids[*handle] = ModuleId::new(index);
        }

        let parts = order
            .iter()
            .map(|handle| {
                let node = &mut nodes[*handle];
                NodeParts {
                    name: mem::take(&mut names[*handle]),
                    kind: node.kind,
                    directives: mem::take(&mut node.directives),
                    package: node.package,
                    parent: node.parent.map(|parent| ids[parent]),
                }
            })
            .collect();

        let mut imports: Vec<_> = imports
            .into_iter()
            .map(|import| (ids[import.importer], ids[import.imported], import.detail))
            .collect();
        imports.sort_unstable();
        imports.dedup();

        Graph::from_parts(parts, imports)
    }

    /// Add an external package or builtin.
    fn add_leaf(&mut self, name: &str, kind: NodeKind) -> Result<NodeHandle, GraphError> {
        if !is_scheme_name(name) {
            return Err(GraphError::InvalidName { name: name.to_string() });
        }
        if let Some(&handle) = self.handles.get(name) {
            return self.expect_kind(handle, name, kind);
        }
        self.insert(name, kind, None)
    }

    /// Find or add the directory at `path` and its ancestors. Nothing is
    /// added unless every ancestor that exists is a directory.
    fn directory(&mut self, path: &str) -> Result<NodeHandle, GraphError> {
        if let Some(&handle) = self.handles.get(path) {
            return self.expect_kind(handle, path, NodeKind::Directory);
        }
        let parent = self.directory(posix_parent(path))?;
        self.insert(path, NodeKind::Directory, Some(parent))
    }

    /// Check that the node with `handle` and `name` has the kind it is being
    /// added as.
    fn expect_kind(
        &self,
        handle: NodeHandle,
        name: &str,
        added: NodeKind,
    ) -> Result<NodeHandle, GraphError> {
        let existing = self.nodes[handle].kind;
        if existing == added {
            Ok(handle)
        } else {
            Err(GraphError::KindConflict { name: name.to_string(), existing, added })
        }
    }

    /// Add a node that does not exist yet.
    fn insert(
        &mut self,
        name: &str,
        kind: NodeKind,
        parent: Option<NodeHandle>,
    ) -> Result<NodeHandle, GraphError> {
        // Every node's index, and the end of the preorder (the node count),
        // must fit in a `u32`.
        if self.nodes.len() >= u32::MAX as usize {
            return Err(GraphError::TooManyNodes);
        }
        let node = PendingNode { kind, package: false, parent, directives: ThinVec::new() };
        let handle = self.nodes.push(node);
        self.handles.insert(name.to_string(), handle);
        Ok(handle)
    }
}

/// Whether `name` has the form `<scheme>:<name>`, with an alphanumeric
/// scheme and a non-empty name.
fn is_scheme_name(name: &str) -> bool {
    name.split_once(':').is_some_and(|(scheme, rest)| {
        !scheme.is_empty()
            && scheme.bytes().all(|byte| byte.is_ascii_alphanumeric())
            && !rest.is_empty()
    })
}

#[cfg(test)]
mod tests {
    use thin_vec::thin_vec;

    use super::*;
    use crate::{
        ImportKind,
        testing::{detail, id, names},
    };

    /// Everything observable about a graph, for comparing two of them.
    fn summary(graph: &Graph) -> Vec<String> {
        graph
            .ids()
            .map(|node| {
                let imports: Vec<_> = graph
                    .imports_of(node)
                    .iter()
                    .map(|imported| (graph.name(*imported), graph.details(node, *imported)))
                    .collect();
                format!(
                    "{} {:?} {:?} children={:?} imports={imports:?}",
                    graph.name(node),
                    graph.node(node).kind(),
                    graph.node(node).directives(),
                    names(graph, graph.children(node).iter().copied()),
                )
            })
            .collect()
    }

    #[test]
    fn an_empty_builder_has_only_the_root() {
        for graph in [GraphBuilder::new().finish(), GraphBuilder::default().finish()] {
            assert_eq!(graph.node_count(), 1);
            assert_eq!(graph.name(ModuleId::ROOT), "");
            assert_eq!(graph.node(ModuleId::ROOT).kind(), NodeKind::Directory);
            assert_eq!(graph.subtree(ModuleId::ROOT), [ModuleId::ROOT]);
            assert!(graph.members(ModuleId::ROOT).is_empty());
            assert_eq!(graph.edges().count(), 0);
            assert_eq!(graph.detail_count(), 0);
        }
    }

    #[test]
    fn the_order_of_insertion_does_not_change_the_graph() {
        let steps: [fn(&mut GraphBuilder); 4] = [
            |builder| {
                let app = builder.add_file("web/src/app.ts").unwrap();
                let api = builder.add_file("web/src/api/client.ts").unwrap();
                builder.add_import(app, api, detail(3, ImportKind::Value));
            },
            |builder| {
                let app = builder.add_file("web/src/app.ts").unwrap();
                let react = builder.add_external("npm:react").unwrap();
                builder.add_import(app, react, detail(1, ImportKind::Value));
                builder.set_directives(app, thin_vec!["use client".to_string()]);
            },
            |builder| {
                let api = builder.add_file("web/src/api/client.ts").unwrap();
                let fs = builder.add_builtin("node:fs").unwrap();
                builder.add_import(api, fs, detail(2, ImportKind::Require));
                builder.add_import(api, fs, detail(1, ImportKind::Type));
            },
            |builder| {
                builder.add_file("packages/ui/button.tsx").unwrap();
            },
        ];
        let build = |steps: &mut dyn Iterator<Item = &fn(&mut GraphBuilder)>| {
            let mut builder = GraphBuilder::new();
            for step in steps {
                step(&mut builder);
            }
            builder.finish()
        };

        let forwards = build(&mut steps.iter());
        let backwards = build(&mut steps.iter().rev());
        assert_eq!(summary(&forwards), summary(&backwards));
        assert_eq!(
            names(&forwards, forwards.ids()),
            [
                "",
                "node:fs",
                "npm:react",
                "packages",
                "packages/ui",
                "packages/ui/button.tsx",
                "web",
                "web/src",
                "web/src/api",
                "web/src/api/client.ts",
                "web/src/app.ts",
            ]
        );
    }

    #[test]
    fn adding_a_node_again_returns_its_handle() {
        let mut builder = GraphBuilder::new();
        let file = builder.add_file("web/src/app.ts").unwrap();
        assert_eq!(builder.add_file("web/src/app.ts"), Ok(file));
        let react = builder.add_external("npm:react").unwrap();
        assert_eq!(builder.add_external("npm:react"), Ok(react));
        let fs = builder.add_builtin("node:fs").unwrap();
        assert_eq!(builder.add_builtin("node:fs"), Ok(fs));
        assert_ne!(NodeHandle::from(file), react);

        let graph = builder.finish();
        assert_eq!(
            names(&graph, graph.ids()),
            ["", "node:fs", "npm:react", "web", "web/src", "web/src/app.ts"]
        );
    }

    #[test]
    fn a_file_importing_itself_gets_no_edge() {
        let mut builder = GraphBuilder::new();
        let file = builder.add_file("a.ts").unwrap();
        builder.add_import(file, file, detail(1, ImportKind::Value));
        let graph = builder.finish();
        assert_eq!(graph.edges().count(), 0);
        assert_eq!(graph.detail_count(), 0);
    }

    #[test]
    fn a_detail_recorded_twice_is_kept_once() {
        let mut builder = GraphBuilder::new();
        let a = builder.add_file("a.ts").unwrap();
        let b = builder.add_file("b.ts").unwrap();
        builder.add_import(a, b, detail(1, ImportKind::Value));
        builder.add_import(a, b, detail(1, ImportKind::Value));
        builder.add_import(a, b, detail(1, ImportKind::Type));
        let graph = builder.finish();
        let [a, b] = ["a.ts", "b.ts"].map(|name| id(&graph, name));
        assert_eq!(
            graph.details(a, b),
            [detail(1, ImportKind::Value), detail(1, ImportKind::Type)]
        );
        assert_eq!(graph.detail_count(), 2);
    }

    #[test]
    fn rejects_paths_that_are_not_normalised() {
        let mut builder = GraphBuilder::new();
        for path in ["", "/a.ts", "a/", "a//b.ts", "./a.ts", "a/./b.ts", "a/../b.ts", ".", ".."] {
            assert_eq!(
                builder.add_file(path),
                Err(GraphError::InvalidPath { path: path.to_string() }),
                "{path:?}"
            );
        }
        assert_eq!(builder.finish().node_count(), 1);

        let error = GraphError::InvalidPath { path: "a//b.ts".to_string() };
        assert_eq!(
            error.to_string(),
            "`a//b.ts` is not a normalised POSIX path relative to the project root"
        );
    }

    #[test]
    fn rejects_names_without_a_scheme() {
        let mut builder = GraphBuilder::new();
        for name in ["react", "@acme/ui", ":react", "npm:", "n/pm:react", ""] {
            let expected = Err(GraphError::InvalidName { name: name.to_string() });
            assert_eq!(builder.add_external(name), expected, "{name:?}");
            assert_eq!(builder.add_builtin(name), expected, "{name:?}");
        }
        assert!(builder.add_external("npm:@acme/ui").is_ok());
        assert!(builder.add_builtin("bun:sqlite").is_ok());

        let error = GraphError::InvalidName { name: "react".to_string() };
        assert_eq!(error.to_string(), "`react` is not a name of the form `<scheme>:<name>`");
    }

    #[test]
    fn rejects_a_name_that_belongs_to_another_kind_of_node() {
        let conflict = |name: &str, existing, added| GraphError::KindConflict {
            name: name.to_string(),
            existing,
            added,
        };

        let mut builder = GraphBuilder::new();
        builder.add_file("a/b.ts").unwrap();
        builder.add_external("npm:react").unwrap();
        assert_eq!(builder.add_file("a"), Err(conflict("a", NodeKind::Directory, NodeKind::File)));
        assert_eq!(
            builder.add_file("a/b.ts/c/d.ts"),
            Err(conflict("a/b.ts", NodeKind::File, NodeKind::Directory))
        );
        assert_eq!(
            builder.add_builtin("npm:react"),
            Err(conflict("npm:react", NodeKind::External, NodeKind::Builtin))
        );
        assert_eq!(
            builder.add_file("npm:react"),
            Err(conflict("npm:react", NodeKind::External, NodeKind::File))
        );

        // Failed additions leave nothing behind.
        let graph = builder.finish();
        assert_eq!(names(&graph, graph.ids()), ["", "a", "a/b.ts", "npm:react"]);

        let error = GraphError::KindConflict {
            name: "npm:react".to_string(),
            existing: NodeKind::External,
            added: NodeKind::File,
        };
        assert_eq!(
            error.to_string(),
            "`npm:react` is already an external package, so it cannot also be a file"
        );
    }

    #[test]
    fn an_import_of_a_package_adds_its_directory_and_marks_it() {
        let mut builder = GraphBuilder::new();
        let main = builder.add_file("cmd/app/main.go").unwrap();
        builder.add_package_import(main, "internal/db", detail(1, ImportKind::Value)).unwrap();
        // Adding a file in the package afterwards, or before, is the same.
        builder.add_file("internal/db/conn.go").unwrap();
        let graph = builder.finish();

        assert_eq!(
            names(&graph, graph.ids()),
            [
                "",
                "cmd",
                "cmd/app",
                "cmd/app/main.go",
                "internal",
                "internal/db",
                "internal/db/conn.go"
            ]
        );
        let [main, db, internal] =
            ["cmd/app/main.go", "internal/db", "internal"].map(|name| id(&graph, name));
        assert!(graph.is_package(db));
        assert!(!graph.is_package(internal));
        assert_eq!(graph.node(db).kind(), NodeKind::Directory);
        assert_eq!(graph.imports_of(main), [db]);
        assert_eq!(graph.details(main, db), [detail(1, ImportKind::Value)]);
    }

    #[test]
    fn the_order_of_package_imports_does_not_change_the_graph() {
        let steps: [fn(&mut GraphBuilder); 3] = [
            |builder| {
                let main = builder.add_file("cmd/main.go").unwrap();
                builder
                    .add_package_import(main, "internal/db", detail(2, ImportKind::Value))
                    .unwrap();
            },
            |builder| {
                let conn = builder.add_file("internal/db/conn.go").unwrap();
                builder
                    .add_package_import(conn, "internal/log", detail(1, ImportKind::Value))
                    .unwrap();
            },
            |builder| {
                let main = builder.add_file("cmd/main.go").unwrap();
                builder
                    .add_package_import(main, "internal/db", detail(1, ImportKind::Type))
                    .unwrap();
            },
        ];
        let build = |steps: &mut dyn Iterator<Item = &fn(&mut GraphBuilder)>| {
            let mut builder = GraphBuilder::new();
            for step in steps {
                step(&mut builder);
            }
            builder.finish()
        };

        let forwards = build(&mut steps.iter());
        let backwards = build(&mut steps.iter().rev());
        assert_eq!(summary(&forwards), summary(&backwards));
        let [main, db] = ["cmd/main.go", "internal/db"].map(|name| id(&forwards, name));
        assert_eq!(
            forwards.details(main, db),
            [detail(1, ImportKind::Type), detail(2, ImportKind::Value)]
        );
    }

    #[test]
    fn a_package_import_recorded_twice_is_kept_once() {
        let mut builder = GraphBuilder::new();
        let main = builder.add_file("main.go").unwrap();
        for _ in 0..2 {
            builder.add_package_import(main, "lib", detail(1, ImportKind::Value)).unwrap();
        }
        let graph = builder.finish();
        assert_eq!(graph.detail_count(), 1);
        assert_eq!(graph.edges().count(), 1);
    }

    #[test]
    fn a_file_importing_its_own_package_gets_no_edge() {
        let mut builder = GraphBuilder::new();
        let file = builder.add_file("lib/a.go").unwrap();
        builder.add_package_import(file, "lib", detail(1, ImportKind::Value)).unwrap();
        let other = builder.add_file("lib/sub/b.go").unwrap();
        builder.add_package_import(other, "lib", detail(1, ImportKind::Value)).unwrap();
        let graph = builder.finish();

        // `lib` is a package only because `lib/sub/b.go` imports it.
        let lib = id(&graph, "lib");
        assert!(graph.is_package(lib));
        assert_eq!(graph.importers_of(lib), [id(&graph, "lib/sub/b.go")]);
        assert_eq!(graph.detail_count(), 1);

        let mut builder = GraphBuilder::new();
        let file = builder.add_file("lib/a.go").unwrap();
        builder.add_package_import(file, "lib", detail(1, ImportKind::Value)).unwrap();
        let graph = builder.finish();
        assert!(!graph.is_package(id(&graph, "lib")));
        assert_eq!(graph.detail_count(), 0);
    }

    #[test]
    fn rejects_a_package_that_is_not_a_directory_path() {
        let mut builder = GraphBuilder::new();
        let main = builder.add_file("main.go").unwrap();
        builder.add_file("lib/a.go").unwrap();
        for path in ["/lib", "lib/", "lib//db", "./lib", "lib/../db", ".."] {
            assert_eq!(
                builder.add_package_import(main, path, detail(1, ImportKind::Value)),
                Err(GraphError::InvalidPath { path: path.to_string() }),
                "{path:?}"
            );
        }
        assert_eq!(
            builder.add_package_import(main, "lib/a.go", detail(1, ImportKind::Value)),
            Err(GraphError::KindConflict {
                name: "lib/a.go".to_string(),
                existing: NodeKind::File,
                added: NodeKind::Directory,
            })
        );
        assert_eq!(
            builder.add_package_import(main, "main.go/sub", detail(1, ImportKind::Value)),
            Err(GraphError::KindConflict {
                name: "main.go".to_string(),
                existing: NodeKind::File,
                added: NodeKind::Directory,
            })
        );

        // Failed imports leave nothing behind.
        let graph = builder.finish();
        assert_eq!(names(&graph, graph.ids()), ["", "lib", "lib/a.go", "main.go"]);
        assert_eq!(graph.detail_count(), 0);
        assert!(!graph.has_packages());
    }

    #[test]
    fn a_package_cannot_be_added_as_a_file() {
        let mut builder = GraphBuilder::new();
        let main = builder.add_file("main.go").unwrap();
        builder.add_package_import(main, "lib", detail(1, ImportKind::Value)).unwrap();
        assert_eq!(
            builder.add_file("lib"),
            Err(GraphError::KindConflict {
                name: "lib".to_string(),
                existing: NodeKind::Directory,
                added: NodeKind::File,
            })
        );
    }
}
