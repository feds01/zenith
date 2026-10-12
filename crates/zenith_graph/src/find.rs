//! Finding the nodes that a path glob or a name pattern picks out, as
//! selectors do.

use zenith_utils::{
    glob::{Glob, NamePattern},
    path::{posix_parent, strip_extension},
};

use crate::{Graph, ModuleId, Node, NodeKind};

impl Graph {
    /// The outermost internal nodes that `glob` matches, in tree order. A
    /// file matches if its path does, or its path without one of
    /// `extensions` (each without its `.`), so `src/types` matches
    /// `src/types.ts` as well as the directory `src/types`. A matched node's
    /// descendants are left out, since it already stands for them.
    pub fn find_paths(&self, glob: &Glob, extensions: &[&str]) -> Vec<ModuleId> {
        let matches = |id: ModuleId| self.node(id).matches_glob(glob, extensions);
        let prefix = glob.literal_prefix();

        // A literal glob names one path, which may be a file's without its
        // extension, so only the children of its parent can match.
        if glob.is_literal() {
            let Some(parent) = self.id(posix_parent(&prefix)) else {
                return Vec::new();
            };
            return self.children(parent).iter().copied().filter(|id| matches(*id)).collect();
        }

        // Otherwise, every match is in the subtree of the literal prefix.
        let Some(start) = self.id(&prefix) else {
            return Vec::new();
        };
        let subtree = self.subtree(start);
        let mut found = Vec::new();
        let mut position = 0;
        while let Some(&id) = subtree.get(position) {
            if matches(id) {
                found.push(id);
                position += self.subtree(id).len();
            } else {
                position += 1;
            }
        }
        found
    }

    /// The external packages or builtins named `<scheme>:<name>`, e.g.
    /// `npm:react`, whose name matches `pattern`, in name order.
    pub fn find_leaves(&self, scheme: &str, pattern: &NamePattern) -> Vec<ModuleId> {
        let prefix = format!("{scheme}:");
        self.ids_with_prefix(&prefix)
            .filter(|id| {
                let node = self.node(*id);
                !node.is_internal() && pattern.is_match(&node.name()[prefix.len()..])
            })
            .collect()
    }
}

impl Node {
    /// Whether `glob` matches this node, as path selectors match: an internal
    /// node by its path, and a file also by its path without one of
    /// `extensions` (each without its `.`). External packages and builtins
    /// have no path, so they never match.
    pub fn matches_glob(&self, glob: &Glob, extensions: &[&str]) -> bool {
        match self.kind() {
            NodeKind::Directory => glob.is_match(self.name()),
            NodeKind::File => {
                glob.is_match(self.name())
                    || strip_extension(self.name(), extensions)
                        .is_some_and(|stem| glob.is_match(stem))
            }
            NodeKind::External | NodeKind::Builtin => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        GraphBuilder,
        testing::{graph_of, names},
    };

    const EXTENSIONS: &[&str] = &["ts", "d.ts", "tsx"];

    /// A graph of web sources, external packages and builtins.
    fn graph() -> Graph {
        graph_of(&[
            ("web/src/app/page.tsx", "web/src/types.ts"),
            ("web/src/app/page.tsx", "web/src/types/user.ts"),
            ("web/src/lib/format.ts", "web/src/env.d.ts"),
            ("web/src/lib/format.test.ts", "web/src/lib/format.ts"),
            ("web/src/lib/format.ts", "npm:react"),
            ("web/src/lib/format.ts", "npm:@acme/legacy"),
            ("web/src/lib/format.ts", "node:fs"),
            ("web/README.md", "web/src/app/page.tsx"),
        ])
    }

    fn find(graph: &Graph, glob: &str) -> Vec<String> {
        let found = graph.find_paths(&Glob::new(glob).unwrap(), EXTENSIONS);
        names(graph, found).into_iter().map(String::from).collect()
    }

    #[test]
    fn globs_find_the_outermost_matches_in_tree_order() {
        let graph = graph();
        assert_eq!(
            find(&graph, "web/src/*"),
            ["web/src/app", "web/src/env.d.ts", "web/src/lib", "web/src/types", "web/src/types.ts"]
        );
        assert_eq!(find(&graph, "web/**"), ["web/README.md", "web/src"]);
        assert_eq!(find(&graph, "**/*.tsx"), ["web/src/app/page.tsx"]);
        assert!(find(&graph, "web/*/missing/*").is_empty());
        assert!(find(&graph, "missing/**").is_empty());
    }

    #[test]
    fn files_match_without_one_of_the_extensions() {
        let graph = graph();
        assert_eq!(find(&graph, "web/src/types"), ["web/src/types", "web/src/types.ts"]);
        assert_eq!(find(&graph, "web/src/lib/format"), ["web/src/lib/format.ts"]);
        assert_eq!(find(&graph, "web/src/env"), ["web/src/env.d.ts"]);
        assert_eq!(
            find(&graph, "web/src/*/f*"),
            ["web/src/lib/format.test.ts", "web/src/lib/format.ts"]
        );
        // `md` isn't one of them.
        assert!(find(&graph, "web/README").is_empty());
        assert_eq!(find(&graph, "web/README.md"), ["web/README.md"]);
        assert!(find(&graph, "missing/types").is_empty());
    }

    #[test]
    fn nodes_match_globs_by_path() {
        let graph = graph();
        let matches = |name: &str, glob: &str| {
            graph.node(graph.id(name).unwrap()).matches_glob(&Glob::new(glob).unwrap(), EXTENSIONS)
        };
        assert!(matches("web/src", "web/*"));
        assert!(matches("web/src/types.ts", "web/src/types.ts"));
        assert!(matches("web/src/types.ts", "web/src/types"));
        assert!(matches("web/src/env.d.ts", "web/src/env"));
        assert!(!matches("web/src/types", "web/src/types.ts"));
        assert!(!matches("web/README.md", "web/README"));
        // Leaves have no path, even if their name looks like one.
        assert!(!matches("npm:react", "npm:react"));
    }

    #[test]
    fn leaves_are_found_by_scheme_and_name() {
        let graph = graph();
        let find = |scheme: &str, pattern: &str| {
            let found = graph.find_leaves(scheme, &NamePattern::new(pattern).unwrap());
            names(&graph, found)
        };
        assert_eq!(find("npm", "*"), ["npm:@acme/legacy", "npm:react"]);
        assert_eq!(find("npm", "@acme/*"), ["npm:@acme/legacy"]);
        assert_eq!(find("node", "*"), ["node:fs"]);
        assert!(find("bun", "*").is_empty());
        assert!(find("npm", "sharp").is_empty());
    }

    #[test]
    fn leaves_are_never_internal_nodes() {
        // A directory and a file whose names look like packages.
        let mut builder = GraphBuilder::new();
        builder.add_file("npm:y/a.ts").unwrap();
        builder.add_external("npm:x").unwrap();
        let graph = builder.finish();
        let found = graph.find_leaves("npm", &NamePattern::new("*").unwrap());
        assert_eq!(names(&graph, found), ["npm:x"]);
    }
}
