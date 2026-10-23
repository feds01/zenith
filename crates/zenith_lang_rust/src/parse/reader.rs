//! Reads a [`ParsedFile`] from the syntax tree that rust-analyzer's parser
//! parses a Rust source into.

use std::{borrow::Cow, cmp::Ordering, iter, ops::Range};

use compact_str::CompactString;
use ra_ap_syntax::{
    AstNode, NodeOrToken, SyntaxElement, SyntaxKind, SyntaxNode, TextRange, WalkEvent,
    ast::{self, HasGenericArgs, HasName},
};
use rustc_hash::{FxHashMap, FxHashSet};
use thin_vec::ThinVec;
use zenith_graph::{ImportDetail, ImportKind, ImportedNames};
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

    /// What each name that a `use` or an `extern crate` brings into scope
    /// stands for, as a path written in the module of the file, with where
    /// its declaration starts. `None` if it stands for more than one path, in
    /// different scopes, or for an item of the file.
    aliases: FxHashMap<CompactString, Option<(CompactString, usize)>>,

    /// The paths in code, each specifier once.
    paths: Vec<ImportDetail>,

    /// The specifiers of [`paths`](Self::paths).
    seen: FxHashSet<CompactString>,

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

    /// Where each inline module that the node being read is in ends, from the
    /// outermost.
    inline: Vec<usize>,

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
            aliases: FxHashMap::default(),
            paths: Vec::new(),
            seen: FxHashSet::default(),
            bound: FxHashSet::default(),
            modules: FxHashSet::default(),
            comments: Vec::new(),
            declaration: 0,
            use_end: 0,
            inline: Vec::new(),
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
        while self.inline.last().is_some_and(|end| range.start >= *end) {
            self.inline.pop();
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
        match node.kind() {
            SyntaxKind::USE => {
                self.declaration = range.start;
                self.use_end = range.end;
                if let Some(tree) = ast::Use::cast(node).and_then(|node| node.use_tree()) {
                    self.use_tree(&tree, "");
                }
            }
            SyntaxKind::EXTERN_CRATE => self.extern_crate(&node),
            SyntaxKind::MODULE => {
                if let Some(module) = ast::Module::cast(node)
                    && module.item_list().is_some()
                    && let Some(name) = module.name()
                {
                    let name: CompactString = self.text(name.syntax()).into();
                    self.bound.insert(name.clone());
                    self.modules.insert(name);
                    self.inline.push(range.end);
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
            self.import(tree.syntax().text_range(), &specifier);
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
        let (specifier, name) = match text.as_ref() {
            "self" if !prefix.is_empty() => (prefix.to_owned(), last_segment(prefix).to_owned()),
            _ => (join(prefix, &text), last_segment(&text).to_owned()),
        };
        match alias.unwrap_or(&name) {
            "_" => {}
            name => {
                self.bound.insert(name.into());
                self.alias(name, &specifier);
            }
        }
        self.import(path.syntax().text_range(), &specifier);
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
        self.declaration = byte_range(node.text_range()).start;
        let alias = match krate.rename() {
            Some(rename) => rename.name().map(|alias| self.text(alias.syntax())),
            None => Some(name_text.clone()),
        };
        if let Some(alias) = alias {
            self.bound.insert(alias.as_ref().into());
            self.alias(&alias, &name_text);
        }
        self.import(name.syntax().text_range(), &name_text);
    }

    /// Record an import of `specifier` that the code at `range` makes.
    fn import(&mut self, range: TextRange, specifier: &str) {
        if let Some(specifier) = self.in_file(specifier) {
            let detail = ImportDetail {
                range: location(range),
                specifier: specifier.as_ref().into(),
                kind: ImportKind::Value,
                names: ImportedNames::All,
            };
            self.imports.push(detail);
            self.declarations.push(self.declaration);
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
    fn text(&self, node: &SyntaxNode) -> Cow<'s, str> {
        strip_blanks(&self.source[byte_range(node.text_range())])
    }

    /// `specifier`, an import of the declaration that starts at
    /// `declaration`, with its first segment replaced by the path that it
    /// stands for, if another declaration brought it into scope: `use
    /// smoltcp::socket::tcp;` then `use tcp::State;` imports
    /// `smoltcp::socket::tcp::State`.
    fn unalias(&self, specifier: &mut CompactString, declaration: usize) {
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
                "" => path.clone(),
                rest => format!("{path}::{rest}").into(),
            };
        }
    }

    /// The file that was read, with its syntax `errors`.
    fn finish(mut self, errors: ThinVec<Located<SyntaxError>>) -> ParsedFile {
        let mut imports = std::mem::take(&mut self.imports);
        for (import, declaration) in imports.iter_mut().zip(&self.declarations) {
            self.unalias(&mut import.specifier, *declaration);
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
        imports.extend(paths);
        imports.sort_by_key(|import| import.range);

        let comments = comments(self.source, &self.comments);
        ParsedFile {
            imports,
            suppressions: Suppression::collect(self.source, comments),
            errors,
            ..ParsedFile::default()
        }
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

/// The last segment of a path.
fn last_segment(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}
