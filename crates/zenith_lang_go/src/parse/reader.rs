//! Reads a [`GoFile`] from the syntax tree that tree-sitter parses a Go source
//! into: the comments and the package clause at the top of the file, then the
//! import declarations, up to the first other top-level node.

use tree_sitter::Node;
use zenith_utils::location::{ByteRange, Located};

use super::{GoComment, GoFile, GoImport, GoSyntaxError, ImportName};

/// The prefix of a line comment that is a build constraint.
const BUILD_CONSTRAINT: &str = "//go:build";

/// Read the file whose syntax tree is `root` from its `source`.
pub(super) fn read(source: &str, root: Node) -> GoFile {
    let mut reader = Reader { source, file: GoFile::default() };
    reader.read(root);
    reader.file
}

/// What has been read of one file so far.
struct Reader<'a> {
    /// The source of the file.
    source: &'a str,

    /// What has been found so far.
    file: GoFile,
}

impl<'a> Reader<'a> {
    /// Read the top-level nodes of `root`, up to the end of the imports.
    fn read(&mut self, root: Node) {
        let mut cursor = root.walk();
        let mut nodes = root.children(&mut cursor).filter(Node::is_named).peekable();

        // 1. Read the comments in front of the package clause, which may hold the build
        //    constraint.
        while let Some(comment) = nodes.next_if(|node| node.kind() == "comment") {
            self.comment(comment);
            if self.file.build_constraint.is_none() {
                self.build_constraint(comment);
            }
        }

        // 2. Read the package clause. Without one, the imports are still read if they
        //    come next.
        let Some(&first) = nodes.peek() else {
            let end = self.source.len();
            self.error(end, end, GoSyntaxError::MissingPackageClause);
            return;
        };
        let mut last = first;
        if first.kind() == "package_clause" || starts_with(first, "package") {
            nodes.next();
            self.header(first);
            self.file.package = named_child(first, "package_identifier")
                .filter(|name| !name.is_missing())
                .map(|name| Located::new(range(name), self.text(name).to_owned()));
        } else {
            let token = first.child(0).unwrap_or(first);
            self.error(token.start_byte(), token.end_byte(), GoSyntaxError::MissingPackageClause);
            if first.kind() != "import_declaration" && !starts_with(first, "import") {
                return;
            }
        }

        // 3. Read the import declarations, and the comments between them. The comments
        //    after the last one are kept only up to the end of its line.
        let mut pending = Vec::new();
        for node in nodes {
            if node.kind() == "comment" {
                pending.push(node);
                continue;
            }
            if node.kind() != "import_declaration" && !starts_with(node, "import") {
                // A node that tree-sitter could not read at all ends the imports
                // too, so report it, as the imports after it are missed.
                if node.is_error() {
                    self.error(node.start_byte(), node.end_byte(), GoSyntaxError::Invalid);
                }
                break;
            }
            pending.drain(..).for_each(|comment| self.comment(comment));
            self.header(node);
            last = node;
        }

        // 4. Keep the comments on the line where the last declaration ends. A block
        //    comment that spans lines ends the line.
        let row = last.end_position().row;
        for comment in pending {
            if comment.start_position().row != row {
                break;
            }
            self.comment(comment);
            if comment.end_position().row != row {
                break;
            }
        }
    }

    /// Read a package clause or an import declaration: its import specs, its
    /// comments and its syntax errors.
    fn header(&mut self, node: Node) {
        self.walk(node, false);
    }

    /// Read the import specs, the comments and the syntax errors in the
    /// subtree of `node`, in source order. A node `in_error` is inside one that
    /// has been reported already.
    fn walk(&mut self, node: Node, in_error: bool) {
        if node.is_missing() {
            if !in_error {
                let at = node.start_byte();
                self.error(at, at, GoSyntaxError::Missing(node.kind().to_owned()));
            }
            return;
        }
        if node.is_error() && !in_error {
            self.error(node.start_byte(), node.end_byte(), GoSyntaxError::Invalid);
        }
        match node.kind() {
            "comment" => return self.comment(node),
            "import_spec" => self.import_spec(node),
            _ => {}
        }
        let in_error = in_error || node.is_error();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, in_error);
        }
    }

    /// Read an import spec, which is a path with an optional name in front of
    /// it. A spec with syntax errors in it is left out.
    fn import_spec(&mut self, spec: Node) {
        // 1. Read the name, if there is one.
        let name = match spec.child_by_field_name("name") {
            None => ImportName::Default,
            Some(name) => match name.kind() {
                "dot" => ImportName::Dot,
                "blank_identifier" => ImportName::Blank,
                _ => ImportName::Alias(self.text(name).to_owned()),
            },
        };

        // 2. Read the path. Escapes are not decoded: no real import path needs one.
        let Some(path_node) = spec.child_by_field_name("path") else {
            return;
        };
        if spec.has_error() {
            return;
        }
        let (start, end) = (path_node.start_byte(), path_node.end_byte());
        let quoted = self.text(path_node);
        let path = match path_node.kind() {
            "interpreted_string_literal" => {
                if named_child(path_node, "escape_sequence").is_some() {
                    return self.error(start, end, GoSyntaxError::EscapedImportPath);
                }
                unquote(quoted).to_owned()
            }
            // Go leaves the carriage returns out of a raw string.
            _ => unquote(quoted).replace('\r', ""),
        };

        // 3. Keep the import.
        if path.is_empty() {
            return self.error(start, end, GoSyntaxError::EmptyImportPath);
        }
        self.file.imports.push(GoImport {
            path,
            range: range(spec),
            path_range: range(path_node),
            name,
        });
    }

    /// Keep the comment `node`. The `\r` of a `\r\n` line ending is not part of
    /// it.
    fn comment(&mut self, node: Node) {
        let text = self.text(node);
        let start = node.start_byte();
        let end = start + text.strip_suffix('\r').unwrap_or(text).len();
        let block = text.starts_with("/*");
        self.file.comments.push(GoComment { range: offsets(start, end), block });
    }

    /// Keep the expression of the comment `node`, if it is a build constraint.
    fn build_constraint(&mut self, node: Node) {
        let comment = self.text(node).trim_end_matches('\r');
        let Some(rest) = comment.strip_prefix(BUILD_CONSTRAINT) else {
            return;
        };
        // The expression is set apart from the directive by a space.
        if !rest.starts_with([' ', '\t']) {
            return;
        }
        let expression = rest.trim_matches([' ', '\t']);
        if expression.is_empty() {
            return;
        }
        let from = node.start_byte() + comment.len() - rest.trim_start_matches([' ', '\t']).len();
        self.file.build_constraint =
            Some(Located::new(offsets(from, from + expression.len()), expression.to_owned()));
    }

    /// Report a syntax error at `start..end`.
    fn error(&mut self, start: usize, end: usize, error: GoSyntaxError) {
        self.file.errors.push(Located::new(offsets(start, end), error));
    }

    /// The source text of `node`.
    fn text(&self, node: Node) -> &'a str {
        self.source.get(node.byte_range()).unwrap_or("")
    }
}

/// Whether the first token of `node` is the keyword `keyword`, as in a
/// declaration that tree-sitter could not read.
fn starts_with(node: Node, keyword: &str) -> bool {
    node.is_error() && node.child(0).is_some_and(|token| token.kind() == keyword)
}

/// The first child of `node` of the kind `kind`.
fn named_child<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|child| child.kind() == kind)
}

/// The text of a string literal between its quotes.
fn unquote(literal: &str) -> &str {
    literal.get(1..literal.len().saturating_sub(1)).unwrap_or("")
}

/// The byte range of `node`.
fn range(node: Node) -> ByteRange {
    offsets(node.start_byte(), node.end_byte())
}

/// A byte range of the source, whose offsets fit in a `u32`.
fn offsets(start: usize, end: usize) -> ByteRange {
    ByteRange::new(start as u32, end as u32)
}
