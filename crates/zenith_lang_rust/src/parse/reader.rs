//! Reads a [`ParsedFile`] from the syntax tree that rust-analyzer's parser
//! parses a Rust source into.

use std::{borrow::Cow, cmp::Ordering, iter, ops::Range};

use compact_str::CompactString;
use ra_ap_syntax::{
    AstNode, NodeOrToken, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, TextRange, WalkEvent,
    ast::{self, HasAttrs, HasGenericArgs, HasName, HasVisibility},
};
use rustc_hash::{FxHashMap, FxHashSet};
use smol_str::SmolStr;
use thin_vec::ThinVec;
use zenith_graph::{Exports, ImportDetail, ImportKind, ImportedName, ImportedNames, ReExport};
use zenith_lang::{ParsedFile, Suppression, SyntaxError};
use zenith_utils::location::{ByteRange, Located};

use super::comments::comments;

/// The primitive types, which have associated functions and constants that
/// are written like paths, such as `u8::MAX`, but are not modules.
const PRIMITIVES: &[&str] = &[
    "bool", "char", "str", "f16", "f32", "f64", "f128", "i8", "i16", "i32", "i64", "i128", "isize",
    "u8", "u16", "u32", "u64", "u128", "usize",
];

/// The macros whose bodies are code that a procedural macro generates for the
/// crates that use it, matched by the last segment of their path. The paths in
/// a body are written for those crates, so are not imports of the crate that
/// holds it.
const QUOTE_MACROS: &[&str] = &["quote", "quote_spanned", "parse_quote", "parse_quote_spanned"];

/// What has been read of one file so far.
pub(super) struct Reader<'s> {
    /// The source of the file.
    source: &'s str,

    /// The leaves of `use` declarations and the `extern crate`s.
    imports: Vec<ImportDetail>,

    /// Where the declaration of each of [`imports`](Self::imports) starts.
    declarations: Vec<usize>,

    /// The name that each of [`imports`](Self::imports) brings into scope, if
    /// it takes a name from the module that it resolves to, rather than the
    /// whole module, as a glob does.
    bindings: Vec<Option<CompactString>>,

    /// What the file exports, with the source of each re-export as the range
    /// of its import.
    exports: Exports<ByteRange>,

    /// Whether the `use` declaration being read is an item of the file, whose
    /// names the file exports.
    exporting: bool,

    /// What each name that a `use` or an `extern crate` brings into scope
    /// stands for, as a path written in the module of the file, with where
    /// its declaration starts. `None` if it stands for more than one path, in
    /// different scopes, or for an item of the file.
    aliases: FxHashMap<CompactString, Option<(CompactString, usize)>>,

    /// The paths in code, each specifier once of each kind.
    paths: Vec<ImportDetail>,

    /// The specifiers of [`paths`](Self::paths), with their kinds.
    seen: FxHashSet<(CompactString, ImportKind)>,

    /// The names that `use` declarations and inline modules bring into scope. A
    /// path in code that starts with one of them is already imported, or is not
    /// a module.
    bound: FxHashSet<CompactString>,

    /// The names of the inline modules. A `use` path that starts with one is
    /// in the file.
    modules: FxHashSet<CompactString>,

    /// The ranges of the comments.
    comments: Vec<Range<usize>>,

    /// Where the `use` declaration or `extern crate` that is being read
    /// starts.
    declaration: usize,

    /// Where the `use` declaration that is being read ends.
    use_end: usize,

    /// Where each inline module that the node being read is in ends, with
    /// its name, from the outermost.
    inline: Vec<(usize, CompactString)>,

    /// Where each item or expression that only tests compile, and that the
    /// node being read is in, ends, from the outermost.
    tests: Vec<usize>,

    /// The modules that the file declares in files of their own, and that
    /// only tests compile, each as its path from the module of the file.
    test_modules: ThinVec<String>,

    /// The last body of a [`QUOTE_MACROS`] macro that was read.
    quoted: Range<usize>,
}

/// A token of a token tree, as a path in it is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    /// A name, or `crate`, `self` or `super`.
    Name,

    /// `::`.
    Separator,

    /// `:`, which may be half of a `::`.
    Colon,

    /// `$`, which starts a macro variable, or `#`, which starts an
    /// interpolation of `quote!`.
    Sigil,

    /// Any other token, or a nested token tree.
    Other,
}

impl<'s> Reader<'s> {
    /// Read the file whose syntax tree is `root`, from its `source`, with its
    /// syntax `errors`.
    pub(super) fn read(
        source: &'s str,
        root: &SyntaxNode,
        errors: ThinVec<Located<SyntaxError>>,
    ) -> ParsedFile {
        let mut reader = Reader {
            source,
            imports: Vec::new(),
            declarations: Vec::new(),
            bindings: Vec::new(),
            exports: Exports::default(),
            exporting: false,
            aliases: FxHashMap::default(),
            paths: Vec::new(),
            seen: FxHashSet::default(),
            bound: FxHashSet::default(),
            modules: FxHashSet::default(),
            comments: Vec::new(),
            declaration: 0,
            use_end: 0,
            inline: Vec::new(),
            tests: Vec::new(),
            test_modules: ThinVec::new(),
            quoted: 0..0,
        };
        // The walk doesn't recurse, so that deeply nested code can't overflow the
        // stack.
        for event in root.preorder_with_tokens() {
            if let WalkEvent::Enter(element) = event {
                reader.visit(element);
            }
        }
        reader.finish(errors)
    }

    /// Read `element`, a node or a token.
    fn visit(&mut self, element: SyntaxElement) {
        let range = byte_range(element.text_range());
        while self.inline.last().is_some_and(|(end, _)| range.start >= *end) {
            self.inline.pop();
        }
        while self.tests.last().is_some_and(|end| range.start >= *end) {
            self.tests.pop();
        }
        let node = match element {
            NodeOrToken::Node(node) => node,
            NodeOrToken::Token(token) => {
                // Doc comments are tokens of their own.
                if matches!(
                    token.kind(),
                    SyntaxKind::COMMENT
                        | SyntaxKind::OUTER_DOC_COMMENT
                        | SyntaxKind::INNER_DOC_COMMENT
                ) {
                    self.comments.push(range);
                }
                return;
            }
        };
        let is_item = node.parent().is_some_and(|parent| parent.kind() == SyntaxKind::SOURCE_FILE);
        if is_item {
            self.exported_item(&node);
        }
        match node.kind() {
            SyntaxKind::USE => {
                self.declaration = range.start;
                self.use_end = range.end;
                self.exporting = is_item;
                if let Some(declaration) = ast::Use::cast(node) {
                    // The declaration is read before its attributes are visited.
                    if declaration.attrs().any(|attr| is_test_attr(&attr)) {
                        self.tests.push(range.end);
                    }
                    if let Some(tree) = declaration.use_tree() {
                        self.use_tree(&tree, "");
                    }
                }
            }
            SyntaxKind::EXTERN_CRATE => self.extern_crate(&node),
            SyntaxKind::MODULE => {
                if let Some(module) = ast::Module::cast(node)
                    && let Some(name) = module.name()
                {
                    let name: CompactString = self.text(name.syntax()).into();
                    if module.item_list().is_some() {
                        self.bound.insert(name.clone());
                        self.modules.insert(name.clone());
                        self.inline.push((range.end, name));
                    } else if self.is_test() || module.attrs().any(|attr| is_test_attr(&attr)) {
                        let inline = self.inline.iter().map(|(_, name)| name.as_str());
                        let path: Vec<&str> = inline.chain([name.as_str()]).collect();
                        self.test_modules.push(path.join("::"));
                    }
                }
            }
            // An attribute that only tests compile what it is on, as `#[cfg(test)]`
            // and `#[test]` do, holds for the rest of that, and `#![cfg(test)]` for the
            // rest of the file or module that it is in.
            SyntaxKind::ATTR => {
                if let Some(attr) = ast::Attr::cast(node.clone())
                    && is_test_attr(&attr)
                    && let Some(parent) = node.parent()
                {
                    self.tests.push(byte_range(parent.text_range()).end);
                }
            }
            SyntaxKind::PATH => self.code_path(&node),
            SyntaxKind::TOKEN_TREE => self.token_tree(&node),
            _ => {}
        }
    }

    /// Read the imports of a `use` tree: its leaves, each as the path that
    /// leads to it from `prefix`, the path of the lists that it is in.
    fn use_tree(&mut self, tree: &ast::UseTree, prefix: &str) {
        let path = tree.path();
        if let Some(list) = tree.use_tree_list() {
            let path = path.map(|path| self.text(path.syntax()));
            let prefix = join(prefix, path.as_deref().unwrap_or_default());
            for tree in list.use_trees() {
                self.use_tree(&tree, &prefix);
            }
        } else if tree.star_token().is_some() {
            // `a::b::*` imports from `a::b`.
            let path = path.map(|path| self.text(path.syntax()));
            let specifier = join(prefix, path.as_deref().unwrap_or_default());
            let range = tree.syntax().text_range();
            if self.exporting {
                self.exports.stars.push((location(range), false));
            }
            self.import(range, &specifier, None);
        } else if let Some(path) = path {
            let alias = tree.rename().map(|rename| match rename.name() {
                Some(name) => self.text(name.syntax()),
                None => Cow::Borrowed("_"),
            });
            self.leaf(&path, prefix, alias.as_deref());
        }
    }

    /// Read the leaf `path` of a `use` tree, which brings its last name, or
    /// `alias`, into scope.
    fn leaf(&mut self, path: &ast::Path, prefix: &str, alias: Option<&str>) {
        // `use a::{self}` imports `a`.
        let text = self.text(path.syntax());
        let (specifier, name, named) = match text.as_ref() {
            "self" if !prefix.is_empty() => {
                (prefix.to_owned(), last_segment(prefix).to_owned(), false)
            }
            _ => (join(prefix, &text), last_segment(&text).to_owned(), true),
        };
        let range = path.syntax().text_range();
        match alias.unwrap_or(&name) {
            "_" => {}
            exported => {
                self.bound.insert(exported.into());
                self.alias(exported, &specifier);
                // A `use` of the file re-exports what it brings into scope to the modules
                // that may see it.
                if self.exporting {
                    self.exports.reexports.push(ReExport {
                        name: exported.into(),
                        source: location(range),
                        imported: Some(name.as_str().into()),
                        type_only: false,
                    });
                }
            }
        }
        let binding = named.then(|| alias.unwrap_or(&name));
        self.import(range, &specifier, binding);
    }

    /// Record that `name`, which the declaration being read brings into
    /// scope, stands for `specifier`.
    fn alias(&mut self, name: &str, specifier: &str) {
        let target =
            self.in_file(specifier).map(|target| (target.as_ref().into(), self.declaration));
        self.aliases
            .entry(name.into())
            .and_modify(|known| {
                if known.as_ref().map(|(path, _)| path) != target.as_ref().map(|(path, _)| path) {
                    *known = None;
                }
            })
            .or_insert(target);
    }

    /// Read `extern crate name;`, which imports the crate.
    fn extern_crate(&mut self, node: &SyntaxNode) {
        let Some(krate) = ast::ExternCrate::cast(node.clone()) else {
            return;
        };
        let Some(name) = krate.name_ref() else {
            return;
        };
        let name_text = self.text(name.syntax());
        // `extern crate self as name;` names the crate itself.
        if name_text == "self" {
            return;
        }
        let range = byte_range(node.text_range());
        self.declaration = range.start;
        // The declaration is read before its attributes are visited.
        if krate.attrs().any(|attr| is_test_attr(&attr)) {
            self.tests.push(range.end);
        }
        let alias = match krate.rename() {
            Some(rename) => rename.name().map(|alias| self.text(alias.syntax())),
            None => Some(name_text.clone()),
        };
        if let Some(alias) = alias {
            self.bound.insert(alias.as_ref().into());
            self.alias(&alias, &name_text);
        }
        self.import(name.syntax().text_range(), &name_text, None);
    }

    /// Record an import of `specifier` that the code at `range` makes, which
    /// takes a name from the module that it resolves to, and brings it into
    /// scope as `binding`, if it has one, rather than the whole module.
    fn import(&mut self, range: TextRange, specifier: &str, binding: Option<&str>) {
        if let Some(specifier) = self.in_file(specifier) {
            let detail = ImportDetail {
                range: location(range),
                specifier: specifier.as_ref().into(),
                kind: self.kind(),
                names: ImportedNames::All,
            };
            self.imports.push(detail);
            self.declarations.push(self.declaration);
            self.bindings.push(binding.map(CompactString::from));
        }
    }

    /// Record the name of `node`, an item of the file, as a name that the file
    /// exports, if it is a type, a function, a constant or a static that is
    /// visible outside the file.
    fn exported_item(&mut self, node: &SyntaxNode) {
        let is_named_item = matches!(
            node.kind(),
            SyntaxKind::FN
                | SyntaxKind::STRUCT
                | SyntaxKind::ENUM
                | SyntaxKind::UNION
                | SyntaxKind::TRAIT
                | SyntaxKind::TYPE_ALIAS
                | SyntaxKind::CONST
                | SyntaxKind::STATIC
        );
        let is_visible = ast::AnyHasVisibility::cast(node.clone())
            .is_some_and(|item| item.visibility().is_some());
        if is_named_item
            && is_visible
            && let Some(name) = ast::AnyHasName::cast(node.clone()).and_then(|item| item.name())
        {
            let name = self.text(name.syntax());
            self.exports.names.push(ImportedName::new(name.as_ref(), false));
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
    /// `use` declaration or of a longer path, or is a single name.
    fn code_path(&mut self, node: &SyntaxNode) {
        let range = node.text_range();
        let is_part = node.parent().is_some_and(|parent| parent.kind() == SyntaxKind::PATH);
        if is_part || byte_range(range).start < self.use_end {
            return;
        }
        let Some(path) = ast::Path::cast(node.clone()) else {
            return;
        };
        if let Some(text) = plain(&path)
            && (path.qualifier().is_some() || text.starts_with("::"))
        {
            self.path(location(range), &text);
        }
    }

    /// Read the paths that the tokens of a macro call, or of an attribute,
    /// make. The parser doesn't read them as paths, since it doesn't know
    /// what the macro does with them. Those of a `quote!` body are not read.
    fn token_tree(&mut self, node: &SyntaxNode) {
        if self.enter_quote(node) {
            return;
        }

        // 1. The tokens, with a `::` as one, though a token tree may have it as two
        //    `:`s.
        let mut tokens: Vec<(Token, TextRange)> = Vec::new();
        for element in node.children_with_tokens() {
            let range = element.text_range();
            let token = match element {
                NodeOrToken::Node(_) => Token::Other,
                NodeOrToken::Token(token) if token.kind().is_trivia() => continue,
                NodeOrToken::Token(token) => match token.kind() {
                    SyntaxKind::IDENT
                    | SyntaxKind::CRATE_KW
                    | SyntaxKind::SELF_KW
                    | SyntaxKind::SUPER_KW => Token::Name,
                    SyntaxKind::COLON2 => Token::Separator,
                    SyntaxKind::COLON => Token::Colon,
                    SyntaxKind::DOLLAR | SyntaxKind::POUND => Token::Sigil,
                    _ => Token::Other,
                },
            };
            match tokens.last_mut() {
                Some((last @ Token::Colon, last_range))
                    if token == Token::Colon && last_range.end() == range.start() =>
                {
                    (*last, *last_range) = (Token::Separator, last_range.cover(range));
                }
                _ => tokens.push((token, range)),
            }
        }
        let is_name =
            |index: usize| tokens.get(index).is_some_and(|(token, _)| *token == Token::Name);
        let is_separator =
            |index: usize| tokens.get(index).is_some_and(|(token, _)| *token == Token::Separator);

        // 2. A path is names between `::`, which may start with one.
        let mut next = 0;
        while next < tokens.len() {
            let first = next;
            let rooted = is_separator(first);
            let mut last = first + usize::from(rooted);
            next += 1;
            if !is_name(last) {
                continue;
            }
            while is_separator(last + 1) && is_name(last + 2) {
                last += 2;
            }
            next = last + 1;

            // `$crate::a` and `$module::a` start with a macro variable, and `#krate::a`
            // with an interpolation.
            let variable = first > 0 && tokens[first - 1].0 == Token::Sigil;
            // A single name after `::` is the end of a longer path, such as `<T>::new`.
            if !variable && last > first + usize::from(rooted) {
                let range = tokens[first].1.cover(tokens[last].1);
                self.path(location(range), &strip_blanks(&self.source[byte_range(range)]));
            }
        }
    }

    /// Record the token tree `tree` as the body that
    /// [`quoted`](Self::quoted) holds if it is the body of one of the
    /// [`QUOTE_MACROS`], and say whether it is that body or in it.
    fn enter_quote(&mut self, tree: &SyntaxNode) -> bool {
        let range = byte_range(tree.text_range());
        if self.quoted.contains(&range.start) {
            return true;
        }
        // A body follows the `!` after the path of its macro, which is a node of its
        // own, or, in another token tree, the tokens of the path.
        let mut before = iter::successors(tree.prev_sibling_or_token(), |element| {
            element.prev_sibling_or_token()
        })
        .filter(|element| !element.kind().is_trivia());
        if before.next().is_none_or(|bang| bang.kind() != SyntaxKind::BANG) {
            return false;
        }
        let is_quote = match before.next() {
            Some(NodeOrToken::Node(path)) => ast::Path::cast(path)
                .and_then(|path| path.segment()?.name_ref())
                .is_some_and(|name| QUOTE_MACROS.contains(&name.text())),
            Some(NodeOrToken::Token(name)) => {
                name.kind() == SyntaxKind::IDENT && QUOTE_MACROS.contains(&name.text())
            }
            None => false,
        };
        if is_quote {
            self.quoted = range;
        }
        is_quote
    }

    /// Record the path in code `text`, at `range`, as an import of the modules
    /// that it names and the first item after them that may be a type or a
    /// constant, which the segments up to it are: `std::time::Instant::now`
    /// imports `std::time::Instant`. A path that starts with a primitive type
    /// is not an import, and nor is one that starts with a type.
    fn path(&mut self, range: ByteRange, text: &str) {
        let body = text.strip_prefix("::").unwrap_or(text);
        let mut end = 0;
        for segment in body.split("::") {
            let is_item = segment.starts_with(char::is_uppercase);
            if is_item && end == 0 {
                break;
            }
            end += segment.len() + if end == 0 { 0 } else { "::".len() };
            if is_item {
                break;
            }
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
        let kind = self.kind();
        if self.seen.insert((specifier.as_ref().into(), kind)) {
            let names = ImportedNames::All;
            let detail = ImportDetail { range, specifier: specifier.as_ref().into(), kind, names };
            self.paths.push(detail);
        }
    }

    /// Whether only tests compile the code being read.
    fn is_test(&self) -> bool {
        !self.tests.is_empty()
    }

    /// The kind of an import in the code being read: a test import if only
    /// tests compile it.
    fn kind(&self) -> ImportKind {
        match self.is_test() {
            true => ImportKind::Test,
            false => ImportKind::Value,
        }
    }

    /// The text of `node`, without whitespace.
    fn text(&self, node: &SyntaxNode) -> Cow<'s, str> {
        strip_blanks(&self.source[byte_range(node.text_range())])
    }

    /// `specifier`, an import of the declaration that starts at
    /// `declaration`, with its first segment replaced by the path that it
    /// stands for, if another declaration brought it into scope: `use
    /// smoltcp::socket::tcp;` then `use tcp::State;` imports
    /// `smoltcp::socket::tcp::State`.
    fn unalias(&self, specifier: &mut SmolStr, declaration: usize) {
        // A few steps follow a chain of names, and stop a cycle.
        for _ in 0..8 {
            let (first, rest) = specifier.split_once("::").unwrap_or((specifier, ""));
            let Some(Some((path, from))) = self.aliases.get(first) else {
                return;
            };
            if *from == declaration || path == first {
                return;
            }
            *specifier = match rest {
                "" => SmolStr::new(path),
                rest => format!("{path}::{rest}").into(),
            };
        }
    }

    /// The file that was read, with its syntax `errors`.
    fn finish(mut self, errors: ThinVec<Located<SyntaxError>>) -> ParsedFile {
        // A name that a path in code starts with, as `m` in `m::f()`, is a module,
        // so the `use` that brought it into scope takes the whole module.
        let qualifiers: FxHashSet<&str> = (self.paths.iter())
            .filter_map(|path| path.specifier.split_once("::").map(|(first, _)| first))
            .filter(|first| self.bound.contains(*first))
            .collect();
        let mut imports = std::mem::take(&mut self.imports);
        for ((import, declaration), binding) in
            imports.iter_mut().zip(&self.declarations).zip(&self.bindings)
        {
            self.unalias(&mut import.specifier, *declaration);
            if let Some(binding) = binding
                && !qualifiers.contains(binding.as_str())
            {
                import.names = taken(&import.specifier);
            }
        }
        // A `use` of an inline module is in the file.
        imports.retain(|import| {
            let first = import.specifier.split("::").next().unwrap_or_default();
            !self.modules.contains(first) || self.aliases.contains_key(first)
        });

        // A path in code whose first segment is a name that a `use` brought into scope
        // is already imported by it.
        let bound = &self.bound;
        let paths = self.paths.into_iter().filter(|path| {
            let first = path.specifier.split("::").next().unwrap_or_default();
            path.specifier.starts_with("::") || !bound.contains(first)
        });
        imports.extend(paths.map(|mut path| {
            path.names = taken(&path.specifier);
            path
        }));
        imports.sort_by_key(|import| import.range);

        // Each re-export's source is the index of its import, if it is kept. A Rust
        // module has no code that importing it runs, so importing a name from it is
        // importing the name from where it is defined, whatever else it has.
        let exports =
            Exports { pass_through: true, ..std::mem::take(&mut self.exports) }.map(|range| {
                let index = imports.binary_search_by_key(&range, |import| import.range);
                u32::try_from(index.ok()?).ok()
            });

        let comments = comments(self.source, &self.comments);
        ParsedFile {
            imports,
            exports,
            suppressions: Suppression::collect(self.source, comments),
            test_modules: self.test_modules,
            errors,
            ..ParsedFile::default()
        }
    }
}

/// Whether `attr` makes what it is on code that only tests compile: `#[test]`,
/// `#[bench]` or another test attribute, such as `#[tokio::test]`, or a
/// `#[cfg]` whose predicate only holds in tests.
fn is_test_attr(attr: &ast::Attr) -> bool {
    match attr.meta() {
        Some(ast::Meta::CfgMeta(cfg)) => {
            cfg.cfg_predicate().is_some_and(|predicate| holds_only_in_tests(&predicate, 0))
        }
        Some(meta) => meta
            .path()
            .and_then(|path| path.segment()?.name_ref())
            .is_some_and(|name| name.text() == "test" || name.text() == "bench"),
        None => false,
    }
}

/// Whether `predicate`, of a `#[cfg]`, only holds when tests are compiled:
/// `test`, an `all` of which one predicate does, or an `any` of which each
/// does. It is nested `depth` deep, and one deeper than a cfg would be is
/// taken to hold anywhere.
fn holds_only_in_tests(predicate: &ast::CfgPredicate, depth: usize) -> bool {
    match predicate {
        ast::CfgPredicate::CfgAtom(atom) => {
            atom.eq_token().is_none()
                && atom.ident_token().is_some_and(|name| name.text() == "test")
        }
        ast::CfgPredicate::CfgComposite(composite) if depth < 16 => {
            let predicates: Vec<_> = composite.cfg_predicates().collect();
            let holds = |predicate: &ast::CfgPredicate| holds_only_in_tests(predicate, depth + 1);
            match composite.keyword().as_ref().map(SyntaxToken::text) {
                Some("all") => predicates.iter().any(holds),
                Some("any") => !predicates.is_empty() && predicates.iter().all(holds),
                _ => false,
            }
        }
        ast::CfgPredicate::CfgComposite(_) => false,
    }
}

/// The text of `path` if it is only names, such as `crate::a::b`, so names
/// modules and items. Generic arguments of its last segment, as in
/// `a::b::<T>()`, are left out. One that starts with a type, as `<T>::f` and
/// `Vec::<T>::new` do, is not.
fn plain(path: &ast::Path) -> Option<String> {
    let mut segments = Vec::new();
    let mut next = Some(path.clone());
    while let Some(path) = next {
        let segment = path.segment()?;
        let is_last = segments.is_empty();
        let has_arguments = segment.generic_arg_list().is_some()
            || segment.parenthesized_arg_list().is_some()
            || segment.ret_type().is_some();
        if segment.type_anchor().is_some() || (has_arguments && !is_last) {
            return None;
        }
        segments.push(segment.name_ref()?.text().to_string());
        next = path.qualifier();
        // A path that starts with `::` names a crate.
        if next.is_none() && segment.coloncolon_token().is_some() {
            segments.push(String::new());
        }
    }
    segments.reverse();
    Some(segments.join("::"))
}

/// The range of `range` in the source, which is shorter than 4 GiB.
fn location(range: TextRange) -> ByteRange {
    ByteRange::new(range.start().into(), range.end().into())
}

/// `range` as a range of bytes of the source.
fn byte_range(range: TextRange) -> Range<usize> {
    range.into()
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

/// The name that the path `specifier` takes from the module that it resolves
/// to, as far as its text tells: the first segment after the first that starts
/// with a capital, which may be a type and the module's item, as `Shape` in
/// `crate::Shape::Circle`, or else the last, as `write` in `crate::write`.
fn taken(specifier: &str) -> ImportedNames {
    let body = specifier.trim_start_matches("::");
    let mut items = body.split("::").skip(1);
    let name = items.find(|segment| segment.starts_with(char::is_uppercase));
    let name = name.unwrap_or_else(|| last_segment(body));
    ImportedNames::of([ImportedName::new(name, false)])
}

/// The last segment of a path.
fn last_segment(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}
