//! Printing trees with box-drawing characters, like the UNIX `tree` command,
//! adapted from `bracketlint`.
//!
//! ```
//! # use zenith_utils::tree_writing::{TreeNode, TreeWriter};
//! let tree = TreeNode::branch(
//!     "web/",
//!     vec![TreeNode::branch("src/", vec![TreeNode::leaf("app.ts")]), TreeNode::leaf("package.json")],
//! );
//! assert_eq!(
//!     TreeWriter::new(&tree).to_string(),
//!     "web/\n├─src/\n│ └─app.ts\n└─package.json\n"
//! );
//! ```

use std::fmt;

/// A node of a tree to print: a label and its children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    /// The text of the node.
    pub label: String,

    /// The nodes below it, in the order they are printed.
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    /// A node without children.
    pub fn leaf(label: impl Into<String>) -> Self {
        Self { label: label.into(), children: Vec::new() }
    }

    /// A node with children.
    pub fn branch(label: impl Into<String>, children: Vec<TreeNode>) -> Self {
        Self { label: label.into(), children }
    }
}

/// Prints a [`TreeNode`] and its descendants through [`fmt::Display`], one
/// line per node, each line ending with `\n`.
#[derive(Debug, Clone)]
pub struct TreeWriter<'t> {
    /// The node to print.
    tree: &'t TreeNode,

    /// What to print before each of its children: the lines of its
    /// ancestors.
    pad: String,
}

impl<'t> TreeWriter<'t> {
    /// A writer of `tree`.
    pub fn new(tree: &'t TreeNode) -> Self {
        Self { tree, pad: String::new() }
    }
}

impl fmt::Display for TreeWriter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.tree.label)?;
        let last = self.tree.children.len().saturating_sub(1);
        for (index, child) in self.tree.children.iter().enumerate() {
            // The last child closes the line of its siblings.
            let (branch, line) = if index == last { ('└', ' ') } else { ('├', '│') };
            let pad = format!("{}{line} ", self.pad);
            write!(f, "{}{branch}─{}", self.pad, TreeWriter { tree: child, pad })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prints_a_leaf_on_its_own() {
        assert_eq!(TreeWriter::new(&TreeNode::leaf("a.ts")).to_string(), "a.ts\n");
    }

    #[test]
    fn continues_the_lines_of_siblings_below() {
        let tree = TreeNode::branch(
            ".",
            vec![
                TreeNode::branch(
                    "packages/",
                    vec![
                        TreeNode::branch("ui/", vec![TreeNode::leaf("button.tsx")]),
                        TreeNode::leaf("index.ts"),
                    ],
                ),
                TreeNode::branch("web/", vec![TreeNode::leaf("app.ts")]),
            ],
        );
        let expected = [
            ".",
            "├─packages/",
            "│ ├─ui/",
            "│ │ └─button.tsx",
            "│ └─index.ts",
            "└─web/",
            "  └─app.ts",
        ];
        assert_eq!(TreeWriter::new(&tree).to_string(), expected.join("\n") + "\n");
    }
}
