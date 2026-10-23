//! Reading `go.mod` files, from the syntax tree of tree-sitter's grammar for
//! them.

use tree_sitter::Node;
use zenith_diagnostics::message;
use zenith_utils::{location::ByteRange, path::is_local_path};

use super::{ModError, Parsed, Replace, ReplaceTarget, Require, unquote};

/// What a `go.mod` file says about its module.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoMod {
    /// The module path, from the `module` directive.
    pub module: Option<String>,

    /// The `require`d modules, in source order.
    pub requires: Vec<Require>,

    /// The `replace`s, in source order.
    pub replaces: Vec<Replace>,
}

impl GoMod {
    /// Read the `go.mod` file `source`.
    pub fn parse(source: &str) -> Parsed<GoMod> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_gomod_orchard::LANGUAGE.into())
            .expect("the go.mod grammar is built for this version of tree-sitter");
        let tree = parser
            .parse(source, None)
            .expect("a parser with a language and no cancellation always parses");

        let mut reader = Reader { source, value: GoMod::default(), errors: Vec::new() };
        let root = tree.root_node();
        let mut cursor = root.walk();
        for directive in root.named_children(&mut cursor) {
            reader.directive(directive);
        }
        Parsed { value: reader.value, errors: reader.errors }
    }
}

/// What has been read of one file so far.
struct Reader<'a> {
    /// The source of the file.
    source: &'a str,

    /// What has been found so far.
    value: GoMod,

    /// The malformed lines found so far.
    errors: Vec<ModError>,
}

impl Reader<'_> {
    /// Read a top-level node, if it is a directive that zenith needs. Any
    /// other directive, or a line that tree-sitter could not read and that
    /// starts with no such directive, is skipped.
    fn directive(&mut self, node: Node) {
        let kind = node.kind();
        let needed = kind == "module_directive"
            || kind.starts_with("require_directive")
            || kind.starts_with("replace_directive")
            || (node.is_error()
                && node
                    .child(0)
                    .is_some_and(|token| matches!(token.kind(), "module" | "require" | "replace")));
        if !needed {
            return;
        }

        // 1. Report what is malformed. A spec with a syntax error in it is left out,
        //    and so is everything in a directive that tree-sitter could not read at
        //    all.
        self.syntax_errors(node);
        if node.is_error() {
            return;
        }

        // 2. Read the module path, or each spec of a single or a block directive.
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.has_error() {
                continue;
            }
            match child.kind() {
                "module_path" => self.module(child),
                "require_spec" => self.require(child),
                "replace_spec" => self.replace(child),
                _ => {}
            }
        }
    }

    /// Read the module path of the `module` directive. A second one is an
    /// error, and does not change it.
    fn module(&mut self, path: Node) {
        let Some(path) = self.operand(path) else { return };
        if self.value.module.is_some() {
            // The error covers the whole directive.
            self.error(path.parent().unwrap_or(path), &message!(go_repeated_module));
            return;
        }
        self.value.module = Some(self.text(path).to_owned());
    }

    /// Read a `require` spec: a module path and a version.
    fn require(&mut self, spec: Node) {
        let path = spec.child_by_field_name("path").and_then(|node| self.operand(node));
        let version = spec.child_by_field_name("version").and_then(|node| self.operand(node));
        let (Some(path), Some(version)) = (path, version) else { return };
        let (path, version) = (self.text(path).to_owned(), self.text(version).to_owned());
        self.value.requires.push(Require { path, version });
    }

    /// Read a `replace` spec: a module path and optionally a version, then
    /// either a directory, or a module path and a version.
    fn replace(&mut self, spec: Node) {
        let field = |name| spec.child_by_field_name(name);
        let Some(old) = field("from_path").and_then(|node| self.operand(node)) else { return };
        let old_version = field("from_version").and_then(|node| self.operand(node));
        let Some(to) = field("to_path").and_then(|node| self.operand(node)) else { return };
        let to_version = field("to_version").and_then(|node| self.operand(node));

        // The grammar reads any target without a version as a file path, so tell
        // a directory from a module path the way Go does.
        let target = self.text(to);
        let new = match (is_local_path(target), to_version) {
            (true, None) => ReplaceTarget::Path(target.to_owned()),
            (true, Some(_)) => return self.error(spec, &message!(go_directory_with_version)),
            (false, Some(version)) => ReplaceTarget::Module {
                path: target.to_owned(),
                version: self.text(version).to_owned(),
            },
            (false, None) => {
                return self.error(spec, &message!(go_replacement_without_version, target));
            }
        };
        let old = self.text(old).to_owned();
        let old_version = old_version.map(|version| self.text(version).to_owned());
        self.value.replaces.push(Replace { old, old_version, new });
    }

    /// `node`, an operand, if its text can be read: a quoted string with an
    /// escape in it is reported instead.
    fn operand<'tree>(&mut self, node: Node<'tree>) -> Option<Node<'tree>> {
        if unquote(self.raw(node)).is_none() {
            self.error(node, &message!(go_escaped_module_path));
            return None;
        }
        Some(node)
    }

    /// Report the syntax errors in the subtree of `node`: each node that
    /// tree-sitter could not read, and each that it found missing.
    fn syntax_errors(&mut self, node: Node) {
        if node.is_missing() {
            let message = match node.is_named() {
                true => message!(go_expected_node, node = node.kind().replace('_', " ")),
                false => message!(go_expected_token, token = node.kind()),
            };
            return self.error(node, &message);
        }
        if node.is_error() {
            return self.error(node, &message!(go_invalid_syntax));
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.has_error() || child.is_missing() {
                self.syntax_errors(child);
            }
        }
    }

    /// Report that `node` is malformed in the way `message` says.
    fn error(&mut self, node: Node, message: &str) {
        self.errors.push(ModError {
            line: node.start_position().row + 1,
            range: ByteRange::new(node.start_byte() as u32, node.end_byte() as u32),
            message: message.to_owned(),
        });
    }

    /// The text of the operand `node`, without its quotes.
    fn text(&self, node: Node) -> &str {
        unquote(self.raw(node)).unwrap_or("")
    }

    /// The source text of `node`, as written.
    fn raw(&self, node: Node) -> &str {
        self.source.get(node.byte_range()).unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = include_str!("fixtures/full.go.mod");

    fn require(path: &str, version: &str) -> Require {
        Require { path: path.to_owned(), version: version.to_owned() }
    }

    fn module(path: &str, version: &str) -> ReplaceTarget {
        ReplaceTarget::Module { path: path.to_owned(), version: version.to_owned() }
    }

    fn local(path: &str) -> ReplaceTarget {
        ReplaceTarget::Path(path.to_owned())
    }

    /// The new side of each `replace` of `source`, which must be clean.
    fn targets(source: &str) -> Vec<ReplaceTarget> {
        let Parsed { value, errors } = GoMod::parse(source);
        assert_eq!(errors, [], "{source:?}");
        value.replaces.into_iter().map(|replace| replace.new).collect()
    }

    #[test]
    fn a_full_file_is_read() {
        let Parsed { value, errors } = GoMod::parse(FULL);
        assert_eq!(errors, []);
        assert_eq!(value.module.as_deref(), Some("github.com/acme/server"));
        assert_eq!(
            value.requires,
            [
                require("github.com/pkg/errors", "v0.9.1"),
                require("golang.org/x/sync", "v0.8.0"),
                require("gopkg.in/yaml.v3", "v3.0.1"),
                require("github.com/stretchr/testify", "v1.9.0"),
                require("github.com/davecgh/go-spew", "v1.1.1"),
            ]
        );
        let replace = |old: &str, old_version: Option<&str>, new| Replace {
            old: old.to_owned(),
            old_version: old_version.map(str::to_owned),
            new,
        };
        assert_eq!(
            value.replaces,
            [
                replace("github.com/pkg/errors", None, local("../errors")),
                replace(
                    "github.com/old/lib",
                    Some("v1.0.0"),
                    module("github.com/new/lib", "v1.0.1")
                ),
                replace("golang.org/x/sync", None, local("./third_party/sync")),
                replace("github.com/b/c", Some("v2.0.0"), local("/opt/c")),
                replace("github.com/win/lib", None, local("..\\win\\lib")),
            ]
        );
    }

    #[test]
    fn an_empty_file_declares_nothing() {
        for source in ["", "\n\n", "// only a comment\n", "\u{feff}"] {
            let parsed = GoMod::parse(source);
            assert_eq!(parsed.value, GoMod::default(), "{source:?}");
            assert_eq!(parsed.errors, [], "{source:?}");
        }
    }

    #[test]
    fn module_paths_may_be_dotless_or_quoted() {
        for (source, path) in [
            ("module myapp\n", "myapp"),
            ("module \"myapp\"\n", "myapp"),
            ("module `my/app`\n", "my/app"),
        ] {
            assert_eq!(GoMod::parse(source).value.module.as_deref(), Some(path), "{source:?}");
        }
    }

    #[test]
    fn replace_forms_are_told_apart() {
        let source = "replace a => b v1\nreplace a v1 => ./b\nreplace a => ../b\nreplace a v0.1.0 => b v0.2.0\n";
        assert_eq!(
            targets(source),
            [module("b", "v1"), local("./b"), local("../b"), module("b", "v0.2.0")]
        );
        let replaces = GoMod::parse(source).value.replaces;
        assert_eq!(replaces[1].old_version.as_deref(), Some("v1"));
        assert_eq!(replaces[3].old_version.as_deref(), Some("v0.1.0"));
    }

    #[test]
    fn windows_paths_are_directories() {
        let source = "replace a => C:\\work\\a\nreplace b => ..\\b\nreplace c => .\\c\n";
        assert_eq!(targets(source), [local("C:\\work\\a"), local("..\\b"), local(".\\c")]);
    }

    #[test]
    fn directives_zenith_has_no_use_for_are_skipped() {
        let source = "module m\ngo 1.22\ntoolchain go1.22.1\ngodebug x=1\nfuture (\n\tx y z\n)\nfuture thing\nrequire a v1\n";
        let Parsed { value, errors } = GoMod::parse(source);
        assert_eq!(errors, []);
        assert_eq!(value.module.as_deref(), Some("m"));
        assert_eq!(value.requires, [require("a", "v1")]);
    }

    #[test]
    fn malformed_lines_are_located_errors() {
        for (source, line, message) in [
            (
                "module m\nreplace x => y\n",
                2,
                "replacement `y` has no version, so it must be a directory path",
            ),
            ("module m\nreplace x => ./y v1\n", 2, "a directory replacement has no version"),
            ("module m\nrequire \"esc\\x41\" v1\n", 2, "escapes in module paths are not supported"),
            ("module m\nrequire x\n", 2, "invalid syntax"),
        ] {
            let Parsed { value, errors } = GoMod::parse(source);
            assert_eq!(value.module.as_deref(), Some("m"), "{source:?}");
            assert_eq!((errors[0].line, errors[0].message.as_str()), (line, message), "{source:?}");
            assert!(value.requires.is_empty() && value.replaces.is_empty(), "{source:?}");
        }

        // Each error points at the source it is about.
        let source = "module m\nrequire \"esc\\x41\" v1\n";
        let error = &GoMod::parse(source).errors[0];
        let (start, end) = (error.range.start() as usize, error.range.end() as usize);
        assert_eq!(&source[start..end], "\"esc\\x41\"");
        assert!(error.to_string().starts_with("line 2: "));
    }

    #[test]
    fn a_malformed_spec_in_a_block_keeps_the_directives_after_it() {
        let source = "module m\nrequire (\n\tbroken\n\tok v1\n)\nrequire b v2\nreplace c => ./c\n";
        let Parsed { value, errors } = GoMod::parse(source);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].line, 3);
        assert_eq!(value.requires, [require("b", "v2")]);
        assert_eq!(value.replaces.len(), 1);
    }

    #[test]
    fn a_repeated_module_directive_keeps_the_first() {
        let Parsed { value, errors } = GoMod::parse("module a\nmodule b\n");
        assert_eq!(value.module.as_deref(), Some("a"));
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].line, 2);
        assert_eq!(errors[0].message, "repeated `module` directive");
    }

    #[test]
    fn nothing_panics_on_garbage() {
        let sources = [
            "(",
            ")",
            "require (",
            "require ()",
            "replace =>",
            "replace => =>",
            "\"",
            "\\",
            "\u{0}",
            "module \u{1F600}",
            "require \u{1F600} \u{1F600}",
            "=>",
            "//",
            "\r\r\n",
        ];
        for source in sources {
            let _ = GoMod::parse(source);
        }
        // Every prefix of a real file, which cuts through words and blocks.
        for end in (0..FULL.len()).filter(|end| FULL.is_char_boundary(*end)) {
            let _ = GoMod::parse(&FULL[..end]);
        }
    }
}
