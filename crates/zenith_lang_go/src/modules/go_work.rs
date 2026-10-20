//! The parser for `go.work` files.

use super::{
    GoDebug, ModError, Parsed, Replace,
    lexer::{Directive, directives},
    shared::{godebug, operands, replace, set_once},
};

/// What a `go.work` file declares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoWork {
    /// The language version from the `go` directive, such as `1.22`.
    pub go: Option<String>,

    /// The toolchain from the `toolchain` directive, such as `go1.22.3`.
    pub toolchain: Option<String>,

    /// The directories of the workspace's modules, from the `use` directives,
    /// as written: relative to the directory of the `go.work` file.
    pub uses: Vec<String>,

    /// The `replace` directives, in source order. They apply to every module
    /// of the workspace.
    pub replaces: Vec<Replace>,

    /// The `godebug` settings, in source order.
    pub godebug: Vec<GoDebug>,
}

impl GoWork {
    /// Read the text of a `go.work` file.
    ///
    /// Never fails: a malformed directive is reported in [`Parsed::errors`]
    /// and left out of the result, and an unknown directive is ignored.
    pub fn parse(source: &str) -> Parsed<GoWork> {
        // 1. Split the source into statements.
        let (statements, mut errors) = directives(source);

        // 2. Read each one by its verb.
        let mut work = GoWork::default();
        for statement in &statements {
            work.read(statement, &mut errors);
        }

        // 3. Report the errors in source order, which the two steps interleave.
        errors.sort_by_key(|error| (error.line, error.range));
        Parsed { value: work, errors }
    }

    /// Fold one statement in.
    fn read(&mut self, statement: &Directive, errors: &mut Vec<ModError>) {
        match statement.verb.text.as_str() {
            "go" => set_once(&mut self.go, statement, errors),
            "toolchain" => set_once(&mut self.toolchain, statement, errors),
            "use" => {
                if let Some([dir]) = operands::<1>(statement, errors) {
                    self.uses.push(dir.to_owned());
                }
            }
            "replace" => self.replaces.extend(replace(statement, errors)),
            "godebug" => self.godebug.extend(godebug(statement, errors)),
            // A directive that this version does not know about.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::ReplaceTarget;

    const FULL: &str = include_str!("fixtures/full.go.work");

    #[test]
    fn a_full_file_is_read_directive_by_directive() {
        let Parsed { value, errors } = GoWork::parse(FULL);
        assert_eq!(errors, []);
        assert_eq!(value.go.as_deref(), Some("1.23.0"));
        assert_eq!(value.toolchain.as_deref(), Some("go1.23.4"));
        assert_eq!(value.uses, ["./api", "./libs/shared", "./tools/gen"]);
        assert_eq!(value.godebug, [GoDebug { key: "default".into(), value: "go1.22".into() }]);
        assert_eq!(value.replaces.len(), 2);
        assert_eq!(value.replaces[0].new, ReplaceTarget::Path("./third_party/sync".into()));
        assert_eq!(value.replaces[1].old_version.as_deref(), Some("v1.0.0"));
        assert_eq!(
            value.replaces[1].new,
            ReplaceTarget::Module { path: "github.com/new/lib".into(), version: "v1.0.1".into() }
        );
    }

    #[test]
    fn a_workspace_may_use_the_current_directory() {
        let parsed = GoWork::parse("use .\nuse (\n\t..\n\t./a\n)\n");
        assert_eq!(parsed.errors, []);
        assert_eq!(parsed.value.uses, [".", "..", "./a"]);
    }

    #[test]
    fn module_directives_are_not_workspace_directives() {
        let parsed = GoWork::parse("module m\nrequire a v1\nuse ./a\n");
        assert_eq!(parsed.errors, []);
        assert_eq!(parsed.value.uses, ["./a"]);
    }

    #[test]
    fn malformed_lines_are_located_errors() {
        let source = "use\nuse a b\nreplace a\ngodebug x\ngo 1.22\nuse (\n\t./ok\n";
        let Parsed { value, errors } = GoWork::parse(source);
        assert_eq!(value.go.as_deref(), Some("1.22"));
        assert_eq!(value.uses, ["./ok"]);
        let lines: Vec<_> = errors.iter().map(|error| error.line).collect();
        // The last error is the block that is never closed, on the last line.
        assert_eq!(lines, [1, 2, 3, 4, 8]);
    }

    #[test]
    fn nothing_panics_on_garbage() {
        for end in (0..FULL.len()).filter(|end| FULL.is_char_boundary(*end)) {
            let _ = GoWork::parse(&FULL[..end]);
        }
        for source in ["use (", "use )", "use \"", "replace (\n=>", "\u{1F600}"] {
            let _ = GoWork::parse(source);
        }
    }
}
