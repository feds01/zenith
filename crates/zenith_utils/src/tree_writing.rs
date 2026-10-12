//! Printing trees, like the UNIX `tree` command, adapted from the Hash
//! compiler's `hash-utils`. The characters that draw the tree are set with a
//! [`TreeWriterConfig`]: box-drawing characters by default, or ASCII, or an
//! indented list.
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

use std::{borrow::Cow, fmt, iter};

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

/// The characters that a [`TreeWriter`] draws a tree with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeWriterConfig {
    /// What to pad the lines of the tree with.
    pub pad: char,

    /// The line that continues past a child to its later siblings.
    pub vertical_line: char,

    /// What joins a child that has later siblings to the line.
    pub middle_intersect: char,

    /// What joins the last child to the line.
    pub end_intersect: char,

    /// What goes between the line and a child's label.
    pub child_prefix: Cow<'static, str>,
}

impl TreeWriterConfig {
    /// Draw trees with Unicode box-drawing characters.
    pub fn unicode() -> Self {
        Self {
            pad: ' ',
            vertical_line: '│',
            middle_intersect: '├',
            end_intersect: '└',
            child_prefix: "─".into(),
        }
    }

    /// Draw trees with Unicode box-drawing characters, and a longer line
    /// before each child.
    pub fn unicode_long() -> Self {
        Self { child_prefix: "──".into(), ..Self::unicode() }
    }

    /// Draw trees with ASCII characters.
    pub fn ascii() -> Self {
        Self {
            pad: ' ',
            vertical_line: '|',
            middle_intersect: '|',
            end_intersect: '`',
            child_prefix: "-".into(),
        }
    }

    /// Draw trees as an indented list, without lines.
    pub fn list() -> Self {
        Self {
            pad: ' ',
            vertical_line: ' ',
            middle_intersect: ' ',
            end_intersect: ' ',
            child_prefix: "-".into(),
        }
    }
}

impl Default for TreeWriterConfig {
    fn default() -> Self {
        Self::unicode()
    }
}

/// Prints a [`TreeNode`] and its descendants through [`fmt::Display`], one
/// line per node, each line ending with `\n`.
#[derive(Debug, Clone)]
pub struct TreeWriter<'t, 'cfg> {
    /// The node to print.
    tree: &'t TreeNode,

    /// What to print before each of its children: the lines of its
    /// ancestors.
    pad: String,

    /// The characters to draw the tree with.
    config: Cow<'cfg, TreeWriterConfig>,
}

impl<'t> TreeWriter<'t, '_> {
    /// A writer of `tree` that draws it with box-drawing characters.
    pub fn new(tree: &'t TreeNode) -> Self {
        Self::with_config(tree, TreeWriterConfig::default())
    }

    /// A writer of `tree` that draws it with the characters of `config`.
    pub fn with_config(tree: &'t TreeNode, config: TreeWriterConfig) -> Self {
        Self { tree, pad: String::new(), config: Cow::Owned(config) }
    }

    /// The writer of the child at `index`, whose lines continue those of its
    /// ancestors.
    fn child(&self, index: usize) -> TreeWriter<'t, '_> {
        // The last child closes the line of its siblings.
        let line = if self.is_last(index) { self.config.pad } else { self.config.vertical_line };
        let indent = iter::repeat_n(self.config.pad, self.config.child_prefix.chars().count());
        let pad = self.pad.chars().chain(iter::once(line)).chain(indent).collect();
        TreeWriter { tree: &self.tree.children[index], pad, config: Cow::Borrowed(&self.config) }
    }

    /// Whether the child at `index` is the last.
    fn is_last(&self, index: usize) -> bool {
        index + 1 == self.tree.children.len()
    }
}

impl fmt::Display for TreeWriter<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.tree.label)?;
        for index in 0..self.tree.children.len() {
            let intersect = if self.is_last(index) {
                self.config.end_intersect
            } else {
                self.config.middle_intersect
            };
            write!(f, "{}{intersect}{}{}", self.pad, self.config.child_prefix, self.child(index))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tree with siblings at two levels.
    fn tree() -> TreeNode {
        TreeNode::branch(
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
        )
    }

    /// `tree` drawn with `config`.
    fn draw(config: TreeWriterConfig) -> String {
        TreeWriter::with_config(&tree(), config).to_string()
    }

    #[test]
    fn prints_a_leaf_on_its_own() {
        assert_eq!(TreeWriter::new(&TreeNode::leaf("a.ts")).to_string(), "a.ts\n");
    }

    #[test]
    fn continues_the_lines_of_siblings_below() {
        let expected = [
            ".",
            "├─packages/",
            "│ ├─ui/",
            "│ │ └─button.tsx",
            "│ └─index.ts",
            "└─web/",
            "  └─app.ts",
        ];
        assert_eq!(TreeWriter::new(&tree()).to_string(), expected.join("\n") + "\n");
    }

    #[test]
    fn draws_with_the_characters_of_its_config() {
        let long = [
            ".",
            "├──packages/",
            "│  ├──ui/",
            "│  │  └──button.tsx",
            "│  └──index.ts",
            "└──web/",
            "   └──app.ts",
        ];
        assert_eq!(draw(TreeWriterConfig::unicode_long()), long.join("\n") + "\n");

        let ascii = [
            ".",
            "|-packages/",
            "| |-ui/",
            "| | `-button.tsx",
            "| `-index.ts",
            "`-web/",
            "  `-app.ts",
        ];
        assert_eq!(draw(TreeWriterConfig::ascii()), ascii.join("\n") + "\n");

        let list = [
            ".",
            " -packages/",
            "   -ui/",
            "     -button.tsx",
            "   -index.ts",
            " -web/",
            "   -app.ts",
        ];
        assert_eq!(draw(TreeWriterConfig::list()), list.join("\n") + "\n");
    }
}
