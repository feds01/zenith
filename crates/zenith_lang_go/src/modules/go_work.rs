//! Reading `go.work` files: the `use` and `replace` directives, which are all
//! that zenith needs from them.
//!
//! A `go.work` file has the line-based syntax of `go.mod`, but there is no
//! published tree-sitter grammar for it, and the `go.mod` one can't read
//! `use`. So this reads the lines itself: a directive is a verb and its
//! operands on one line, or a verb and `(`, then one statement per line up to
//! a `)` line. Operands are words or quoted strings, and `//` starts a
//! comment.

use zenith_diagnostics::message;
use zenith_utils::{location::ByteRange, path::is_local_path};

use super::{ModError, Parsed, Replace, ReplaceTarget, unquote};

/// What a `go.work` file says about its workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoWork {
    /// The directories of the modules that the workspace uses, as written,
    /// relative to the file's directory, such as `./api`.
    pub uses: Vec<String>,

    /// The `replace`s, in source order. They apply to every module of the
    /// workspace.
    pub replaces: Vec<Replace>,
}

impl GoWork {
    /// Read the `go.work` file `source`.
    pub fn parse(source: &str) -> Parsed<GoWork> {
        let mut reader = Reader::default();
        let mut block: Option<(String, Line)> = None;
        let mut offset = 0;
        for (index, text) in source.split_inclusive('\n').enumerate() {
            let line = Line { number: index + 1, start: offset, text };
            offset += text.len();
            let Some(words) = reader.words(&line) else { continue };

            // 1. Inside a block, each line is a statement of its verb, up to `)`.
            if let Some((verb, _)) = &block {
                if words == [")"] {
                    block = None;
                } else {
                    let verb = verb.clone();
                    reader.statement(&verb, &words, &line);
                }
                continue;
            }

            // 2. Otherwise a line is a statement, or the start of a block.
            match words.as_slice() {
                [] => {}
                [verb, open] if open == "(" => block = Some((verb.clone(), line)),
                [verb, operands @ ..] => reader.statement(verb, operands, &line),
            }
        }
        if let Some((verb, line)) = block {
            reader.error(&line, &message!(go_unclosed_block, verb));
        }
        Parsed { value: reader.value, errors: reader.errors }
    }
}

/// One line of the source.
struct Line<'a> {
    /// Its number, counting from 1.
    number: usize,

    /// The offset of its first byte in the source.
    start: usize,

    /// Its text, with its line ending.
    text: &'a str,
}

/// What has been read of one file so far.
#[derive(Default)]
struct Reader {
    /// What has been found so far.
    value: GoWork,

    /// The malformed lines found so far.
    errors: Vec<ModError>,
}

impl Reader {
    /// The words of `line` up to its comment, with `(`, `)` and `=>` as words
    /// of their own, and quoted strings unquoted. `None` if a string is never
    /// closed, or has an escape in it, which has been reported.
    fn words(&mut self, line: &Line) -> Option<Vec<String>> {
        let text = line.text.trim_end_matches(['\n', '\r']).trim_start_matches('\u{feff}');
        let mut words = Vec::new();
        let mut rest = text.trim_start();
        while !rest.is_empty() && !rest.starts_with("//") {
            let length = match rest.as_bytes()[0] {
                quote @ (b'"' | b'`') => match rest[1..].find(quote as char) {
                    Some(end) => end + 2,
                    None => {
                        self.error(line, &message!(go_unterminated_string));
                        return None;
                    }
                },
                b'(' | b')' => 1,
                _ if rest.starts_with("=>") => 2,
                _ => rest
                    .find(|c: char| c.is_whitespace() || c == '(' || c == ')')
                    .map_or(rest.len(), |end| rest[..end].find("=>").unwrap_or(end)),
            };
            let Some(word) = unquote(&rest[..length]) else {
                self.error(line, &message!(go_escaped_module_path));
                return None;
            };
            words.push(word.to_owned());
            rest = rest[length..].trim_start();
        }
        Some(words)
    }

    /// Read the statement of `verb` with `operands`. A verb other than `use`
    /// and `replace` is skipped.
    fn statement(&mut self, verb: &str, operands: &[String], line: &Line) {
        match (verb, operands) {
            ("use", [dir]) if !is_punctuation(dir) => self.value.uses.push(dir.clone()),
            ("use", _) => self.error(line, &message!(go_use_operands)),
            ("replace", operands) => {
                if let Some(replace) = self.replace(operands, line) {
                    self.value.replaces.push(replace);
                }
            }
            _ => {}
        }
    }

    /// Read the operands of a `replace`: a module path and optionally a
    /// version, `=>`, then either a directory, or a module path and a
    /// version.
    fn replace(&mut self, operands: &[String], line: &Line) -> Option<Replace> {
        let Some(arrow) = operands.iter().position(|word| word == "=>") else {
            self.error(line, &message!(go_replace_without_arrow));
            return None;
        };
        let (old, new) = (&operands[..arrow], &operands[arrow + 1..]);
        if old.iter().chain(new).any(|word| is_punctuation(word)) {
            self.error(line, &message!(go_replace_punctuation));
            return None;
        }
        let (old, old_version) = match old {
            [path] => (path.clone(), None),
            [path, version] => (path.clone(), Some(version.clone())),
            _ => {
                self.error(line, &message!(go_replace_old));
                return None;
            }
        };
        let new = match new {
            [path] if is_local_path(path) => ReplaceTarget::Path(path.clone()),
            [path, version] if !is_local_path(path) => {
                ReplaceTarget::Module { path: path.clone(), version: version.clone() }
            }
            _ => {
                self.error(line, &message!(go_replace_new));
                return None;
            }
        };
        Some(Replace { old, old_version, new })
    }

    /// Report that `line` is malformed in the way `message` says.
    fn error(&mut self, line: &Line, message: &str) {
        let end = line.start + line.text.trim_end_matches(['\n', '\r']).len();
        self.errors.push(ModError {
            line: line.number,
            range: ByteRange::new(line.start as u32, end as u32),
            message: message.to_owned(),
        });
    }
}

/// Whether `word` is one of the punctuation words, which no operand is.
fn is_punctuation(word: &str) -> bool {
    matches!(word, "(" | ")" | "=>")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = include_str!("fixtures/full.go.work");

    #[test]
    fn a_full_file_is_read() {
        let Parsed { value, errors } = GoWork::parse(FULL);
        assert_eq!(errors, []);
        assert_eq!(value.uses, ["./api", "./libs/shared", "./tools/gen"]);
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
    fn other_directives_are_skipped() {
        let parsed = GoWork::parse("go 1.22\nmodule m\nrequire a v1\nfuture (\n\tx\n)\nuse ./a\n");
        assert_eq!(parsed.errors, []);
        assert_eq!(parsed.value.uses, ["./a"]);
    }

    #[test]
    fn words_split_at_punctuation_and_comments() {
        let parsed = GoWork::parse("replace a=>./b // why\nuse \"./c d\" // quoted\r\n");
        assert_eq!(parsed.errors, []);
        assert_eq!(parsed.value.uses, ["./c d"]);
        assert_eq!(parsed.value.replaces[0].new, ReplaceTarget::Path("./b".into()));
    }

    #[test]
    fn malformed_lines_are_located_errors() {
        let source = "use\nuse a b\nreplace a\nreplace a => b\nuse \"open\nuse (\n\t./ok\n";
        let Parsed { value, errors } = GoWork::parse(source);
        assert_eq!(value.uses, ["./ok"]);
        let lines: Vec<_> = errors.iter().map(|error| error.line).collect();
        // The last error is the block that is never closed, at the line that opens it.
        assert_eq!(lines, [1, 2, 3, 4, 5, 6]);
        let error = &errors[1];
        let (start, end) = (error.range.start() as usize, error.range.end() as usize);
        assert_eq!(&source[start..end], "use a b");
    }

    #[test]
    fn nothing_panics_on_garbage() {
        for end in (0..FULL.len()).filter(|end| FULL.is_char_boundary(*end)) {
            let _ = GoWork::parse(&FULL[..end]);
        }
        for source in ["use (", "use )", "use \"", "replace (\n=>", "\u{1F600}", "=", "=>=>"] {
            let _ = GoWork::parse(source);
        }
    }
}
