//! Reads a [`TerraformFile`] from the syntax tree that tree-sitter parses an
//! HCL source into: the references in the top-level blocks of the dialect,
//! the comments at the top level and in those blocks, and the syntax errors
//! anywhere in the file.

use tree_sitter::Node;
use zenith_utils::location::{ByteRange, Located};

use super::{
    Dialect, Reference, ReferenceKind, TerraformComment, TerraformFile, TerraformSyntaxError,
};

/// The escapes of an HCL string: a backslash, and the doubled `$${` and `%%{`
/// that stand for a literal `${` and `%{`.
const ESCAPES: &[&str] = &["\\", "$${", "%%{"];

/// Read the file whose syntax tree is `root` from its `source`.
pub(super) fn read(source: &str, root: Node, dialect: Dialect) -> TerraformFile {
    let mut reader = Reader { source, dialect, file: TerraformFile::default() };

    // 1. Read the references from the top-level blocks, and the comments around
    //    them.
    reader.top_level(root);

    // 2. Report the syntax errors, wherever they are. Only the subtrees with an
    //    error in them are walked.
    if root.has_error() {
        reader.errors(root);
    }
    reader.file
}

/// What is read from a kind of block: the attribute whose value is its
/// references.
struct BlockSpec {
    /// The kind of the references in it.
    kind: ReferenceKind,

    /// The name of the attribute that holds them.
    attribute: &'static str,

    /// Whether the attribute is a list of references, rather than one.
    list: bool,
}

impl BlockSpec {
    /// What is read from the blocks of `dialect` whose type is `name`, if
    /// anything is.
    fn of(dialect: Dialect, name: &str) -> Option<Self> {
        let (kind, attribute, list) = match (dialect, name) {
            (Dialect::Terraform, "module") => (ReferenceKind::Module, "source", false),
            (Dialect::Terragrunt, "terraform") => {
                (ReferenceKind::TerragruntSource, "source", false)
            }
            (Dialect::Terragrunt, "dependency") => {
                (ReferenceKind::Dependency, "config_path", false)
            }
            (Dialect::Terragrunt, "dependencies") => (ReferenceKind::Dependency, "paths", true),
            _ => return None,
        };
        Some(Self { kind, attribute, list })
    }
}

/// What has been read of one file so far.
struct Reader<'a> {
    /// The source of the file.
    source: &'a str,

    /// The kind of file it is.
    dialect: Dialect,

    /// What has been found so far.
    file: TerraformFile,
}

impl<'a> Reader<'a> {
    /// Read the top-level nodes of `node`, the root or its body: the blocks
    /// that hold references, and the comments between them.
    fn top_level(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "comment" => self.comment(child),
                "body" => self.top_level(child),
                "block" => self.block(child),
                _ => {}
            }
        }
    }

    /// Read a top-level block, if it is one whose references are read: the
    /// attribute that holds them, and every comment in it.
    fn block(&mut self, block: Node) {
        let Some(spec) =
            block.named_child(0).and_then(|name| BlockSpec::of(self.dialect, self.text(name)))
        else {
            return;
        };
        let mut cursor = block.walk();
        for child in block.children(&mut cursor) {
            if child.kind() != "body" {
                self.comments(child);
                continue;
            }
            let mut cursor = child.walk();
            for item in child.children(&mut cursor) {
                let name = item.named_child(0).map(|name| self.text(name));
                if item.kind() == "attribute" && name == Some(spec.attribute) {
                    self.attribute(item, &spec, block);
                }
                self.comments(item);
            }
        }
    }

    /// Read the value of `attribute`, the attribute that `spec` names in
    /// `block`, as its references. An attribute with syntax errors in it is
    /// left out.
    fn attribute(&mut self, attribute: Node, spec: &BlockSpec, block: Node) {
        let Some(expression) = code(attribute).nth(1).filter(|_| !attribute.has_error()) else {
            return;
        };
        if !spec.list {
            return self.value(expression, spec.kind, attribute, block);
        }
        let tuple =
            only_child(expression, "collection_value").and_then(|value| only_child(value, "tuple"));
        let Some(tuple) = tuple else {
            return self.non_literal(expression, spec.kind);
        };
        let mut cursor = tuple.walk();
        for element in tuple.named_children(&mut cursor).filter(|node| node.kind() == "expression")
        {
            self.value(element, spec.kind, attribute, block);
        }
    }

    /// Read `expression` as a reference of `kind` in `attribute` of `block`, if
    /// it is a string literal.
    fn value(&mut self, expression: Node, kind: ReferenceKind, attribute: Node, block: Node) {
        // 1. Only a string literal can be read without evaluating it.
        let literal = only_child(expression, "literal_value")
            .and_then(|value| only_child(value, "string_lit"));
        let Some(literal) = literal else {
            return self.non_literal(expression, kind);
        };

        // 2. Read the string between its quotes. Escapes are not decoded: no real
        //    module source or path needs one.
        let (start, end) = (literal.start_byte(), literal.end_byte());
        let value = unquote(self.text(literal));
        if ESCAPES.iter().any(|escape| value.contains(escape)) {
            return self.error(start, end, TerraformSyntaxError::EscapedReference(kind));
        }
        if value.is_empty() {
            return self.error(start, end, TerraformSyntaxError::EmptyReference(kind));
        }

        // 3. Keep the reference.
        self.file.references.push(Reference {
            kind,
            value: value.to_owned(),
            range: range(literal),
            attribute: range(attribute),
            block: range(block),
        });
    }

    /// Record `expression` as a reference of `kind` that is not a literal.
    fn non_literal(&mut self, expression: Node, kind: ReferenceKind) {
        self.file.non_literal.push(Located::new(range(expression), kind));
    }

    /// Keep every comment in the subtree of `node`, in source order.
    fn comments(&mut self, node: Node) {
        if node.kind() == "comment" {
            return self.comment(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.comments(child);
        }
    }

    /// Keep the comment `node`. The `\r` of a `\r\n` line ending is not part of
    /// it.
    fn comment(&mut self, node: Node) {
        let text = self.text(node);
        let start = node.start_byte();
        let end = start + text.strip_suffix('\r').unwrap_or(text).len();
        let block = text.starts_with("/*");
        self.file.comments.push(TerraformComment { range: offsets(start, end), block });
    }

    /// Report the syntax errors in the subtree of `node`, in source order. What
    /// is inside an error is not reported again.
    fn errors(&mut self, node: Node) {
        if node.is_missing() {
            let at = node.start_byte();
            let missing = missing_token(node.kind()).unwrap_or(node.kind());
            return self.error(at, at, TerraformSyntaxError::Missing(missing.to_owned()));
        }
        if node.is_error() {
            return self.error(node.start_byte(), node.end_byte(), TerraformSyntaxError::Invalid);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(Node::has_error) {
            self.errors(child);
        }
    }

    /// Report a syntax error at `start..end`, keeping the errors in source
    /// order.
    fn error(&mut self, start: usize, end: usize, error: TerraformSyntaxError) {
        let errors = &mut self.file.errors;
        let at = errors.partition_point(|error| error.range.start() as usize <= start);
        errors.insert(at, Located::new(offsets(start, end), error));
    }

    /// The source text of `node`.
    fn text(&self, node: Node) -> &'a str {
        self.source.get(node.byte_range()).unwrap_or("")
    }
}

/// The token that a missing node of the kind `kind` stands for, if the grammar
/// names it as a node: the ends of strings and interpolations.
fn missing_token(kind: &str) -> Option<&'static str> {
    match kind {
        "quoted_template_end" => Some("\""),
        "template_interpolation_end" | "template_directive_end" => Some("}"),
        _ => None,
    }
}

/// The named children of `node` that are not comments.
fn code<'tree>(node: Node<'tree>) -> impl Iterator<Item = Node<'tree>> {
    (0..node.named_child_count())
        .filter_map(move |index| node.named_child(index as u32))
        .filter(|child| child.kind() != "comment")
}

/// The child of `node` of the kind `kind`, if it is the only child that is not
/// a comment, as the value of a string literal is the only child of its
/// expression, unlike that of `"a"[0]`.
fn only_child<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut children = code(node);
    let child = children.next().filter(|child| child.kind() == kind)?;
    children.next().is_none().then_some(child)
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
