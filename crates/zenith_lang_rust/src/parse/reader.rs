//! Reads a [`ParsedFile`] from the syntax tree that tree-sitter parses a Rust
//! source into.

use std::{borrow::Cow, cmp::Ordering, ops::Range};

use compact_str::CompactString;
use rustc_hash::FxHashSet;
use thin_vec::ThinVec;
use tree_sitter::Node;
use zenith_graph::{ImportDetail, ImportKind, ImportedNames};
use zenith_lang::{ParsedFile, Suppression, SyntaxError};
use zenith_utils::location::{ByteRange, Located};

use super::comments::comments;

/// The primitive types, which have associated functions and constants that
/// are written like paths, such as `u8::MAX`, but are not modules.
const PRIMITIVES: &[&str] = &[
    "bool", "char", "str", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16",
    "u32", "u64", "u128", "usize",
];

/// What has been read of one file so far.
pub(super) struct Reader<'s> {
    /// The source of the file.
    source: &'s str,

    /// The leaves of `use` declarations and the `extern crate`s.
    imports: Vec<ImportDetail>,

    /// The paths in code, each specifier once.
    paths: Vec<ImportDetail>,

    /// The specifiers of [`paths`](Self::paths).
    seen: FxHashSet<CompactString>,

    /// The names that `use` declarations and inline modules bring into scope. A
    /// path in code that starts with one of them is already imported, or is not
    /// a module.
    bound: FxHashSet<CompactString>,

    /// The ranges of the comments.
    comments: Vec<Range<usize>>,

    /// The syntax errors.
    errors: ThinVec<Located<SyntaxError>>,

    /// Where the last error ends. Errors inside it are part of it.
    reported_until: usize,

    /// Where the `use` declaration that is being read ends.
    use_end: usize,

    /// Where each inline module that the node being read is in ends, from the
    /// outermost.
    inline: Vec<usize>,

    /// The node that is the `path` of the scoped path that was just read, which
    /// is part of that path, and not a path of its own.
    nested: Option<usize>,
}

impl<'s> Reader<'s> {
    /// Read the file whose syntax tree is `root`, from its `source`.
    pub(super) fn read(source: &'s str, root: Node<'s>) -> ParsedFile {
        let mut reader = Reader {
            source,
            imports: Vec::new(),
            paths: Vec::new(),
            seen: FxHashSet::default(),
            bound: FxHashSet::default(),
            comments: Vec::new(),
            errors: ThinVec::new(),
            reported_until: 0,
            use_end: 0,
            inline: Vec::new(),
            nested: None,
        };
        reader.walk(root);
        reader.finish()
    }

    /// Visit every node of the tree below `root`, in source order. It doesn't
    /// recurse, so that deeply nested code can't overflow the stack.
    fn walk(&mut self, root: Node<'s>) {
        let mut cursor = root.walk();
        loop {
            if self.visit(cursor.node()) && cursor.goto_first_child() {
                continue;
            }
            while !cursor.goto_next_sibling() {
                if !cursor.goto_parent() {
                    return;
                }
            }
        }
    }

    /// Read `node`. Returns whether to read the nodes below it too.
    fn visit(&mut self, node: Node<'s>) -> bool {
        while self.inline.last().is_some_and(|end| node.start_byte() >= *end) {
            self.inline.pop();
        }
        if node.is_missing() {
            self.error(node, missing(node.kind()));
            return false;
        }
        if node.is_error() {
            self.error(node, Cow::Borrowed("invalid syntax"));
        }
        match node.kind() {
            "line_comment" | "block_comment" => {
                self.comments.push(node.byte_range());
                return false;
            }
            "use_declaration" => {
                self.use_end = node.end_byte();
                if let Some(argument) = node.child_by_field_name("argument") {
                    self.use_tree(argument, "");
                }
            }
            "extern_crate_declaration" => self.extern_crate(node),
            "mod_item" => {
                if let Some(name) = node.child_by_field_name("name")
                    && node.child_by_field_name("body").is_some()
                {
                    self.bound.insert(self.text(name).into());
                    self.inline.push(node.end_byte());
                }
            }
            "scoped_identifier" | "scoped_type_identifier" => self.scoped(node),
            "token_tree" => self.token_tree(node),
            _ => {}
        }
        true
    }

    /// Record a syntax error at `node`, unless it is inside the last one.
    fn error(&mut self, node: Node<'s>, message: Cow<'static, str>) {
        if node.start_byte() >= self.reported_until {
            self.reported_until = node.end_byte();
            let error = SyntaxError { message };
            self.errors.push(Located::new(node_range(node), error));
        }
    }

    /// Read the imports of a `use` tree: its leaves, each as the path that
    /// leads to it from `prefix`, the path of the lists that it is in.
    fn use_tree(&mut self, node: Node<'s>, prefix: &str) {
        match node.kind() {
            "scoped_use_list" => {
                let path = node.child_by_field_name("path").map(|path| self.text(path));
                let prefix = join(prefix, path.as_deref().unwrap_or_default());
                if let Some(list) = node.child_by_field_name("list") {
                    self.use_tree(list, &prefix);
                }
            }
            "use_list" => {
                let mut cursor = node.walk();
                for item in node.named_children(&mut cursor).filter(|item| !item.is_extra()) {
                    self.use_tree(item, prefix);
                }
            }
            "use_as_clause" => {
                let alias = node.child_by_field_name("alias").map(|alias| self.text(alias));
                if let Some(path) = node.child_by_field_name("path") {
                    self.leaf(path, prefix, alias.as_deref());
                }
            }
            // `a::b::*` imports from `a::b`.
            "use_wildcard" => {
                let mut cursor = node.walk();
                let path = node.named_children(&mut cursor).find(|path| !path.is_extra());
                let specifier = join(prefix, &path.map(|path| self.text(path)).unwrap_or_default());
                self.import(node, &specifier);
            }
            "identifier" | "crate" | "self" | "super" | "scoped_identifier" => {
                self.leaf(node, prefix, None);
            }
            _ => {}
        }
    }

    /// Read the leaf `path` of a `use` tree, which brings its last name, or
    /// `alias`, into scope.
    fn leaf(&mut self, path: Node<'s>, prefix: &str, alias: Option<&str>) {
        // `use a::{self}` imports `a`.
        let (specifier, name) = match path.kind() {
            "self" if !prefix.is_empty() => (prefix.to_owned(), last_segment(prefix).to_owned()),
            _ => {
                let text = self.text(path);
                (join(prefix, &text), last_segment(&text).to_owned())
            }
        };
        match alias.unwrap_or(&name) {
            "_" => {}
            name => {
                self.bound.insert(name.into());
            }
        }
        self.import(path, &specifier);
    }

    /// Read `extern crate name;`, which imports the crate.
    fn extern_crate(&mut self, node: Node<'s>) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let alias = node.child_by_field_name("alias").unwrap_or(name);
        let (name_text, alias_text) = (self.text(name), self.text(alias));
        // `extern crate self as name;` names the crate itself.
        if name_text != "self" {
            self.bound.insert(alias_text.as_ref().into());
            self.import(name, &name_text);
        }
    }

    /// Record an import of `specifier` that `node` makes.
    fn import(&mut self, node: Node<'s>, specifier: &str) {
        if let Some(specifier) = self.in_file(specifier) {
            let detail = ImportDetail {
                range: node_range(node),
                specifier: specifier.as_ref().into(),
                kind: ImportKind::Value,
                names: ImportedNames::All,
            };
            self.imports.push(detail);
        }
    }

    /// The path `specifier`, which is written in the inline modules that the
    /// code is in, as it would be written in the module of the file. `None`
    /// if it only reaches into them, so is in the file.
    fn in_file<'a>(&self, specifier: &'a str) -> Option<Cow<'a, str>> {
        let leading = specifier.split("::").take_while(|s| matches!(*s, "self" | "super")).count();
        let depth = self.inline.len();
        if depth == 0 || leading == 0 {
            return Some(specifier.into());
        }
        let supers = specifier.split("::").take(leading).filter(|s| *s == "super").count();
        let rest = specifier.split("::").skip(leading);
        let mut path = match supers.cmp(&depth) {
            Ordering::Less => return None,
            Ordering::Equal => vec!["self"],
            Ordering::Greater => vec!["super"; supers - depth],
        };
        path.extend(rest);
        (path != ["self"]).then(|| path.join("::").into())
    }

    /// Read a path in code, such as `crate::a::b()`, unless it is part of a
    /// `use` declaration or of a longer path.
    fn scoped(&mut self, node: Node<'s>) {
        let path = node.child_by_field_name("path");
        let is_part = self.nested == Some(node.id());
        let nested = path.filter(|path| is_scoped(*path));
        self.nested = nested.map(|path| path.id());
        if !is_part && node.start_byte() >= self.use_end && is_plain(node) {
            self.path(node_range(node), &self.text(node));
        }
    }

    /// Read the paths that the tokens of a macro call, or of an attribute,
    /// make. Tree-sitter doesn't read them as paths, since it doesn't know
    /// what the macro does with them.
    fn token_tree(&mut self, node: Node<'s>) {
        let mut cursor = node.walk();
        let tokens: Vec<Node<'s>> = node.children(&mut cursor).collect();
        let is_name = |token: Option<&Node<'s>>| {
            token.is_some_and(|token| {
                matches!(token.kind(), "identifier" | "crate" | "self" | "super")
            })
        };
        let is_separator =
            |token: Option<&Node<'s>>| token.is_some_and(|token| token.kind() == "::");

        let mut next = 0;
        while next < tokens.len() {
            // A path is names between `::`, which may start with one.
            let first = next;
            let rooted = is_separator(tokens.get(first));
            let mut last = first + usize::from(rooted);
            next += 1;
            if !is_name(tokens.get(last)) {
                continue;
            }
            while is_separator(tokens.get(last + 1)) && is_name(tokens.get(last + 2)) {
                last += 2;
            }
            next = last + 1;

            // `$crate::a` and `$module::a` start with a macro variable.
            let variable = first > 0 && matches!(tokens[first - 1].kind(), "$" | "metavariable");
            // A single name after `::` is the end of a longer path, such as `<T>::new`.
            if !variable && last > first + usize::from(rooted) {
                let range = tokens[first].start_byte()..tokens[last].end_byte();
                self.path(
                    ByteRange::new(range.start as u32, range.end as u32),
                    &strip_blanks(&self.source[range]),
                );
            }
        }
    }

    /// Record the path in code `text`, at `range`, as an import of the modules
    /// that it names, which are the segments before the first that may be a
    /// type or a constant. A path that starts with a primitive type is not
    /// an import.
    fn path(&mut self, range: ByteRange, text: &str) {
        let body = text.strip_prefix("::").unwrap_or(text);
        let mut end = 0;
        for segment in
            body.split("::").take_while(|segment| !segment.starts_with(char::is_uppercase))
        {
            end += segment.len() + if end == 0 { 0 } else { "::".len() };
        }
        let first = body.split("::").next().unwrap_or_default();
        let is_module = matches!(first, "crate" | "self" | "super")
            || text.len() > body.len()
            || (first.starts_with(|c: char| c.is_lowercase() || c == '_')
                && !PRIMITIVES.contains(&first));
        if end == 0 || !is_module {
            return;
        }
        let specifier = &text[..text.len() - body.len() + end];
        let Some(specifier) = self.in_file(specifier) else {
            return;
        };
        if self.seen.insert(specifier.as_ref().into()) {
            let names = ImportedNames::All;
            let detail = ImportDetail {
                range,
                specifier: specifier.as_ref().into(),
                kind: ImportKind::Value,
                names,
            };
            self.paths.push(detail);
        }
    }

    /// The text of `node`, without whitespace.
    fn text(&self, node: Node<'s>) -> Cow<'s, str> {
        strip_blanks(&self.source[node.byte_range()])
    }

    /// The file that was read.
    fn finish(self) -> ParsedFile {
        // A path in code whose first segment is a name that a `use` brought into scope
        // is already imported by it.
        let bound = &self.bound;
        let paths = self.paths.into_iter().filter(|path| {
            let first = path.specifier.split("::").next().unwrap_or_default();
            path.specifier.starts_with("::") || !bound.contains(first)
        });
        let mut imports: Vec<_> = self.imports.into_iter().chain(paths).collect();
        imports.sort_by_key(|import| import.range);

        let comments = comments(self.source, &self.comments);
        ParsedFile {
            imports,
            suppressions: Suppression::collect(self.source, comments),
            errors: self.errors,
            ..ParsedFile::default()
        }
    }
}

/// What is missing, as a syntax error says it: a token, such as `;`, or a
/// kind of node, such as `identifier`.
fn missing(kind: &str) -> Cow<'static, str> {
    match kind.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        true => format!("expected {}", kind.replace('_', " ")).into(),
        false => format!("expected `{kind}`").into(),
    }
}

/// Whether `node` is a path with a scope.
fn is_scoped(node: Node) -> bool {
    matches!(node.kind(), "scoped_identifier" | "scoped_type_identifier")
}

/// Whether the scoped path `node` is only names, so its text is the path. One
/// that starts with a type, as `<T>::f` and `Vec::<T>::new` do, is not.
fn is_plain(mut node: Node) -> bool {
    loop {
        match node.child_by_field_name("path") {
            // The path starts with `::`.
            None => return true,
            Some(path) if is_scoped(path) => node = path,
            Some(path) => return matches!(path.kind(), "identifier" | "crate" | "self" | "super"),
        }
    }
}

/// The range of `node`, which must be in a source shorter than 4 GiB.
fn node_range(node: Node) -> ByteRange {
    ByteRange::new(node.start_byte() as u32, node.end_byte() as u32)
}

/// `text` without its whitespace, which may be between the segments of a
/// path that is written over several lines.
fn strip_blanks(text: &str) -> Cow<'_, str> {
    match text.contains(char::is_whitespace) {
        true => text.chars().filter(|c| !c.is_whitespace()).collect::<String>().into(),
        false => text.into(),
    }
}

/// `path`, the path of a list, followed by `segment`.
fn join(path: &str, segment: &str) -> String {
    match (path.is_empty(), segment.is_empty()) {
        (true, _) => segment.to_owned(),
        (_, true) => path.to_owned(),
        _ => format!("{path}::{segment}"),
    }
}

/// The last segment of a path.
fn last_segment(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}
