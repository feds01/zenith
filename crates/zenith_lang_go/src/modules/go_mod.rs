//! The parser for `go.mod` files.

use super::{
    GoDebug, ModError, ModuleVersion, Parsed, Replace,
    lexer::{Directive, directives},
    shared::{godebug, operands, replace, set_once},
};

/// What a `go.mod` file declares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoMod {
    /// The module path from the `module` directive, such as
    /// `github.com/user/repo` or `myapp`.
    pub module: Option<String>,

    /// The language version from the `go` directive, such as `1.22`.
    pub go: Option<String>,

    /// The toolchain from the `toolchain` directive, such as `go1.22.3`.
    pub toolchain: Option<String>,

    /// The `require` directives, in source order.
    pub requires: Vec<Require>,

    /// The `replace` directives, in source order.
    pub replaces: Vec<Replace>,

    /// The `exclude` directives, in source order.
    pub excludes: Vec<ModuleVersion>,

    /// The `retract` directives, in source order.
    pub retracts: Vec<Retract>,

    /// The `godebug` settings, in source order.
    pub godebug: Vec<GoDebug>,

    /// The packages named by `tool` directives.
    pub tools: Vec<String>,

    /// The directories named by `ignore` directives.
    pub ignores: Vec<String>,
}

/// A `require path version` directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Require {
    /// The module path of the dependency.
    pub path: String,

    /// The version that is required, such as `v1.2.3`.
    pub version: String,

    /// Whether the line has the comment `// indirect`: no package of the
    /// module imports the dependency itself.
    pub indirect: bool,
}

/// A `retract` directive: one version, or a range of versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retract {
    /// The retracted version, or the lowest of the retracted range.
    pub low: String,

    /// The highest version of the retracted range `[low, high]`. `None` for a
    /// single version.
    pub high: Option<String>,
}

impl GoMod {
    /// Read the text of a `go.mod` file.
    ///
    /// Never fails: a malformed directive is reported in [`Parsed::errors`]
    /// and left out of the result, and an unknown directive is ignored.
    pub fn parse(source: &str) -> Parsed<GoMod> {
        // 1. Split the source into statements.
        let (statements, mut errors) = directives(source);

        // 2. Read each one by its verb.
        let mut go_mod = GoMod::default();
        for statement in &statements {
            go_mod.read(statement, &mut errors);
        }

        // 3. Report the errors in source order, which the two steps interleave.
        errors.sort_by_key(|error| (error.line, error.range));
        Parsed { value: go_mod, errors }
    }

    /// Fold one statement in.
    fn read(&mut self, statement: &Directive, errors: &mut Vec<ModError>) {
        match statement.verb.text.as_str() {
            "module" => {
                let Some([path]) = operands::<1>(statement, errors) else { return };
                if self.module.is_some() {
                    errors.push(statement.error("repeated `module` directive"));
                } else {
                    self.module = Some(path.to_owned());
                }
            }
            "go" => set_once(&mut self.go, statement, errors),
            "toolchain" => set_once(&mut self.toolchain, statement, errors),
            "require" => {
                if let Some([path, version]) = operands::<2>(statement, errors) {
                    let indirect = statement.comment.as_deref().is_some_and(is_indirect);
                    self.requires.push(Require {
                        path: path.to_owned(),
                        version: version.to_owned(),
                        indirect,
                    });
                }
            }
            "replace" => self.replaces.extend(replace(statement, errors)),
            "exclude" => {
                if let Some([path, version]) = operands::<2>(statement, errors) {
                    self.excludes
                        .push(ModuleVersion { path: path.to_owned(), version: version.to_owned() });
                }
            }
            "retract" => self.retracts.extend(retract(statement, errors)),
            "godebug" => self.godebug.extend(godebug(statement, errors)),
            "tool" => {
                if let Some([path]) = operands::<1>(statement, errors) {
                    self.tools.push(path.to_owned());
                }
            }
            "ignore" => {
                if let Some([dir]) = operands::<1>(statement, errors) {
                    self.ignores.push(dir.to_owned());
                }
            }
            // A directive that this version does not know about.
            _ => {}
        }
    }
}

/// Whether a line comment marks a requirement as indirect: it starts with the
/// word `indirect`, which `go` may follow with `;` and a note.
fn is_indirect(comment: &str) -> bool {
    comment
        .strip_prefix("indirect")
        .is_some_and(|rest| rest.chars().next().is_none_or(|c| c.is_whitespace() || c == ';'))
}

/// A `retract` statement: `retract v1.0.0` or `retract [v1.0.0, v1.2.0]`.
fn retract(statement: &Directive, errors: &mut Vec<ModError>) -> Option<Retract> {
    let args: Vec<_> =
        statement.args.iter().map(|token| (token.is_any_punct(), token.text.as_str())).collect();
    match args.as_slice() {
        [(false, version)] => Some(Retract { low: (*version).to_owned(), high: None }),
        [(true, "["), (false, low), (true, ","), (false, high), (true, "]")] => {
            Some(Retract { low: (*low).to_owned(), high: Some((*high).to_owned()) })
        }
        _ => {
            errors.push(
                statement.error("`retract` takes a version, or a range such as [v1.0.0, v1.1.0]"),
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::ReplaceTarget;

    const FULL: &str = include_str!("fixtures/full.go.mod");

    fn require(path: &str, version: &str, indirect: bool) -> Require {
        Require { path: path.to_owned(), version: version.to_owned(), indirect }
    }

    fn module(path: &str, version: &str) -> ReplaceTarget {
        ReplaceTarget::Module { path: path.to_owned(), version: version.to_owned() }
    }

    fn local(path: &str) -> ReplaceTarget {
        ReplaceTarget::Path(path.to_owned())
    }

    #[test]
    fn a_full_file_is_read_directive_by_directive() {
        let Parsed { value, errors } = GoMod::parse(FULL);
        assert_eq!(errors, []);
        assert_eq!(value.module.as_deref(), Some("github.com/acme/server"));
        assert_eq!(value.go.as_deref(), Some("1.23.0"));
        assert_eq!(value.toolchain.as_deref(), Some("go1.23.4"));
        assert_eq!(
            value.godebug,
            [
                GoDebug { key: "default".into(), value: "go1.21".into() },
                GoDebug { key: "panicnil".into(), value: "1".into() },
                GoDebug { key: "asynctimerchan".into(), value: "0".into() },
            ]
        );
        assert_eq!(
            value.tools,
            ["golang.org/x/tools/cmd/stringer", "github.com/a/gen", "github.com/b/lint"]
        );
        assert_eq!(value.ignores, ["./node_modules", "./testdata"]);
        assert_eq!(
            value.requires,
            [
                require("github.com/pkg/errors", "v0.9.1", false),
                require("golang.org/x/sync", "v0.8.0", false),
                require("gopkg.in/yaml.v3", "v3.0.1", true),
                require("github.com/stretchr/testify", "v1.9.0", true),
                require("github.com/davecgh/go-spew", "v1.1.1", false),
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
        assert_eq!(
            value.excludes,
            [
                ModuleVersion { path: "golang.org/x/net".into(), version: "v0.1.0".into() },
                ModuleVersion { path: "golang.org/x/text".into(), version: "v0.3.0".into() },
            ]
        );
        let retract = |low: &str, high: Option<&str>| Retract {
            low: low.to_owned(),
            high: high.map(str::to_owned),
        };
        assert_eq!(
            value.retracts,
            [
                retract("v1.0.0", None),
                retract("v1.1.0", Some("v1.2.0")),
                retract("v1.3.0", None),
                retract("v1.4.0", Some("v1.5.0")),
            ]
        );
    }

    #[test]
    fn an_empty_file_declares_nothing() {
        for source in ["", "\n\n", "// only a comment\n", "\u{feff}"] {
            let parsed = GoMod::parse(source);
            assert_eq!(parsed.value, GoMod::default(), "{source:?}");
            assert_eq!(parsed.errors, []);
        }
    }

    #[test]
    fn a_dotless_module_path_is_read_as_it_is() {
        let parsed = GoMod::parse("module myapp\n");
        assert_eq!(parsed.value.module.as_deref(), Some("myapp"));
        let parsed = GoMod::parse("module \"myapp\"\n");
        assert_eq!(parsed.value.module.as_deref(), Some("myapp"));
    }

    #[test]
    fn the_module_directive_may_be_a_block() {
        let parsed = GoMod::parse("module (\n\tmyapp\n)\n");
        assert_eq!(parsed.value.module.as_deref(), Some("myapp"));
        assert_eq!(parsed.errors, []);
    }

    #[test]
    fn replace_forms_are_told_apart() {
        let source = "replace a => b v1\nreplace a v1 => ./b\nreplace a=>../b\nreplace a v0.1.0 => b v0.2.0\n";
        let Parsed { value, errors } = GoMod::parse(source);
        assert_eq!(errors, []);
        let news: Vec<_> = value.replaces.iter().map(|r| r.new.clone()).collect();
        assert_eq!(news, [module("b", "v1"), local("./b"), local("../b"), module("b", "v0.2.0")]);
        assert_eq!(value.replaces[1].old_version.as_deref(), Some("v1"));
        assert_eq!(value.replaces[3].old_version.as_deref(), Some("v0.1.0"));
    }

    #[test]
    fn windows_paths_are_directories() {
        let source = "replace a => C:\\work\\a\nreplace b => ..\\b\nreplace c => .\\c\n";
        let parsed = GoMod::parse(source);
        assert_eq!(parsed.errors, []);
        let news: Vec<_> = parsed.value.replaces.iter().map(|r| r.new.clone()).collect();
        assert_eq!(news, [local("C:\\work\\a"), local("..\\b"), local(".\\c")]);
    }

    #[test]
    fn indirect_is_only_the_word_indirect() {
        let source = "require (\n\ta v1 // indirect\n\tb v1 //indirect\n\tc v1 // indirectly\n\td v1 // indirect; why\n\te v1\n)\n";
        let flags: Vec<_> =
            GoMod::parse(source).value.requires.iter().map(|r| r.indirect).collect();
        assert_eq!(flags, [true, true, false, true, false]);
    }

    #[test]
    fn unknown_directives_and_blocks_are_skipped() {
        let source = "module m\nfuture (\n\tx y z\n)\nfuture thing\ngo 1.22\n";
        let parsed = GoMod::parse(source);
        assert_eq!(parsed.errors, []);
        assert_eq!(parsed.value.module.as_deref(), Some("m"));
        assert_eq!(parsed.value.go.as_deref(), Some("1.22"));
    }

    #[test]
    fn malformed_lines_are_errors_with_a_location_and_do_not_stop_the_parse() {
        let source = "module a b\nrequire x\nrequire ok v1\nreplace x y\nreplace x => y\nreplace x => y v1 z\nreplace x v1 v2 => y v1\nreplace x => ./y v1\nreplace x\nexclude x\nretract\nretract [v1, v2\nretract [v1 v2]\ngodebug nonsense\ngodebug =x\n";
        let Parsed { value, errors } = GoMod::parse(source);
        assert_eq!(value.requires, [require("ok", "v1", false)]);
        assert_eq!(value.module, None);
        let lines: Vec<_> = errors.iter().map(|error| error.line).collect();
        assert_eq!(lines, [1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
        // The location points at the line's own bytes.
        let error = &errors[1];
        let (start, end) = (error.range.start() as usize, error.range.end() as usize);
        assert_eq!(&source[start..end], "require x");
        assert!(error.to_string().starts_with("line 2: "));
    }

    #[test]
    fn repeated_single_value_directives_keep_the_first() {
        let source = "module a\nmodule b\ngo 1.21\ngo 1.22\ntoolchain t1\ntoolchain t2\n";
        let parsed = GoMod::parse(source);
        assert_eq!(parsed.value.module.as_deref(), Some("a"));
        assert_eq!(parsed.value.go.as_deref(), Some("1.21"));
        assert_eq!(parsed.value.toolchain.as_deref(), Some("t1"));
        assert_eq!(parsed.errors.len(), 3);
    }

    #[test]
    fn punctuation_is_not_an_operand() {
        let parsed = GoMod::parse("module (\nrequire ( a v1\nrequire a )\nreplace a => => b v1\n");
        assert_eq!(parsed.value.module, None);
        assert_eq!(parsed.value.requires, []);
        assert_eq!(parsed.value.replaces, []);
        assert!(!parsed.errors.is_empty());
    }

    #[test]
    fn errors_come_in_source_order() {
        let parsed = GoMod::parse("require a\n\"unclosed\nrequire b\nmodule\n");
        let lines: Vec<_> = parsed.errors.iter().map(|error| error.line).collect();
        assert_eq!(lines, [1, 2, 3, 4]);
    }

    #[test]
    fn nothing_panics_on_garbage() {
        let sources = [
            "(",
            ")",
            "((",
            "require (",
            "require ()",
            "replace =>",
            "replace => =>",
            "retract [",
            "retract ]",
            "retract [,]",
            "godebug =",
            "\"",
            "\\",
            "\u{0}",
            "module \u{1F600}",
            "require \u{1F600} \u{1F600}",
            "=>",
            "= >",
            "//",
            "a //\u{1F600}",
            "\r\r\n",
        ];
        for source in sources {
            let _ = GoMod::parse(source);
        }
        // Every prefix of a real file, which cuts through tokens and blocks.
        for end in (0..FULL.len()).filter(|end| FULL.is_char_boundary(*end)) {
            let _ = GoMod::parse(&FULL[..end]);
        }
    }
}
