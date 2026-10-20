//! The lexical layer that `go.mod` and `go.work` share: lines of whitespace
//! separated words, `//` comments, quoted strings, and `verb ( ... )` blocks.
//! It yields one [`Directive`] per statement, whatever its verb.

use zenith_utils::location::ByteRange;

use super::ModError;

/// A word, a quoted string, or a piece of punctuation on a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Token {
    /// The text, with the quotes of a quoted string removed and its escapes
    /// resolved.
    pub text: String,

    /// Whether it was a quoted string, which is never punctuation.
    pub quoted: bool,

    /// Where it is in the source.
    pub range: ByteRange,
}

impl Token {
    /// Whether this is the unquoted punctuation `punct`, such as `(` or `=>`.
    pub fn is_punct(&self, punct: &str) -> bool {
        !self.quoted && self.text == punct
    }

    /// Whether this is some unquoted punctuation, which no operand can be.
    pub fn is_any_punct(&self) -> bool {
        !self.quoted && matches!(self.text.as_str(), "(" | ")" | "[" | "]" | "," | "=>")
    }
}

/// One statement: a verb and its operands, from a line of its own or from a
/// line of a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Directive {
    /// The verb: the first word of a statement, or of the block it is in.
    pub verb: Token,

    /// The operands, which follow the verb on a one-line statement and make
    /// up the whole line in a block.
    pub args: Vec<Token>,

    /// The text of the `//` comment at the end of the line, without the `//`
    /// and trimmed. Such as `indirect`.
    pub comment: Option<String>,

    /// The line the statement is on, counting from 1.
    pub line: usize,

    /// The bytes of the operands, or of the verb when there are none.
    pub range: ByteRange,
}

impl Directive {
    /// An error about this statement.
    pub fn error(&self, message: impl Into<String>) -> ModError {
        ModError { line: self.line, range: self.range, message: message.into() }
    }
}

/// An offset that is clamped to what a [`ByteRange`] holds, so a source that is
/// larger than 4 GiB is mislocated rather than rejected.
fn offset(offset: usize) -> u32 {
    u32::try_from(offset).unwrap_or(u32::MAX)
}

/// The range of the bytes `start..end` of a line that starts at `base`.
fn range(base: usize, start: usize, end: usize) -> ByteRange {
    ByteRange::new(offset(base + start), offset(base + end))
}

/// Split a source into its statements, and the errors in the way it is
/// written: a quoted string without an end, a stray `)`, a block without one.
pub(super) fn directives(source: &str) -> (Vec<Directive>, Vec<ModError>) {
    let mut directives = Vec::new();
    let mut errors = Vec::new();
    // The verb of the block that we are in.
    let mut block: Option<Token> = None;

    let (source, mut base) = match source.strip_prefix('\u{feff}') {
        Some(source) => (source, '\u{feff}'.len_utf8()),
        None => (source, 0),
    };
    let mut last_line = 0;
    for (index, line) in source.split('\n').enumerate() {
        let number = index + 1;
        last_line = number;
        let line_base = base;
        base += line.len() + 1;

        let (tokens, comment) = match lex_line(line, line_base, number) {
            Ok(lexed) => lexed,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        let (Some(first), Some(last)) = (tokens.first(), tokens.last()) else {
            continue;
        };
        let span = ByteRange::new(first.range.start(), last.range.end());

        // 1. Inside a block, every line is an operand list, up to the `)`.
        if let Some(verb) = &block {
            if tokens.len() == 1 && first.is_punct(")") {
                block = None;
            } else if first.is_punct("(") || first.is_punct(")") {
                errors.push(ModError {
                    line: number,
                    range: span,
                    message: format!("unexpected `{}`", first.text),
                });
            } else {
                directives.push(Directive {
                    verb: verb.clone(),
                    args: tokens,
                    comment,
                    line: number,
                    range: span,
                });
            }
            continue;
        }

        // 2. Outside one, a line is a verb, and either `(` or its operands.
        if first.is_any_punct() {
            errors.push(ModError {
                line: number,
                range: first.range,
                message: format!("unexpected `{}`", first.text),
            });
            continue;
        }
        let (verb, args) = (first.clone(), tokens[1..].to_vec());
        if args.len() == 1 && args[0].is_punct("(") {
            block = Some(verb);
            continue;
        }
        let range = args
            .last()
            .map_or(verb.range, |last| ByteRange::new(verb.range.start(), last.range.end()));
        directives.push(Directive { verb, args, comment, line: number, range });
    }

    if let Some(verb) = block {
        errors.push(ModError {
            line: last_line,
            range: verb.range,
            message: format!("`{}` block is never closed with `)`", verb.text),
        });
    }
    (directives, errors)
}

/// Whether a bare word ends at `bytes[i]`.
fn ends_word(bytes: &[u8], i: usize) -> bool {
    match bytes[i] {
        b' ' | b'\t' | b'\r' | b'(' | b')' | b'[' | b']' | b',' | b'"' | b'`' => true,
        b'/' => bytes.get(i + 1) == Some(&b'/'),
        b'=' => bytes.get(i + 1) == Some(&b'>'),
        _ => false,
    }
}

/// Split one line, which starts at `base` in the source, into its tokens and
/// its `//` comment.
fn lex_line(
    line: &str,
    base: usize,
    number: usize,
) -> Result<(Vec<Token>, Option<String>), ModError> {
    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    let mut comment = None;
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        match bytes[i] {
            b' ' | b'\t' | b'\r' => i += 1,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                comment = Some(line[i + 2..].trim().to_owned());
                break;
            }
            b'(' | b')' | b'[' | b']' | b',' => {
                i += 1;
                tokens.push(Token {
                    text: line[start..i].to_owned(),
                    quoted: false,
                    range: range(base, start, i),
                });
            }
            b'=' if bytes.get(i + 1) == Some(&b'>') => {
                i += 2;
                tokens.push(Token {
                    text: "=>".to_owned(),
                    quoted: false,
                    range: range(base, start, i),
                });
            }
            quote @ (b'"' | b'`') => {
                // A string ends at the same quote. Only a `"` string has escapes.
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += if quote == b'"' && bytes[i] == b'\\' { 2 } else { 1 };
                }
                if i >= bytes.len() {
                    return Err(ModError {
                        line: number,
                        range: range(base, start, bytes.len()),
                        message: "string is never closed".to_owned(),
                    });
                }
                let inner = &line[start + 1..i];
                i += 1;
                let text = if quote == b'"' { unescape(inner) } else { inner.to_owned() };
                tokens.push(Token { text, quoted: true, range: range(base, start, i) });
            }
            _ => {
                // The first byte is never a delimiter, and a multibyte
                // character has none in it, so the word ends on a boundary.
                i += 1;
                while i < bytes.len() && !ends_word(bytes, i) {
                    i += 1;
                }
                tokens.push(Token {
                    text: line[start..i].to_owned(),
                    quoted: false,
                    range: range(base, start, i),
                });
            }
        }
    }
    Ok((tokens, comment))
}

/// The text of a `"` string without its quotes: `\"`, `\\`, `\n`, `\t` and
/// `\r` are resolved, and any other escaped character stands for itself.
fn unescape(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(directive: &Directive) -> Vec<&str> {
        directive.args.iter().map(|token| token.text.as_str()).collect()
    }

    #[test]
    fn a_statement_is_a_verb_and_its_operands() {
        let (found, errors) = directives("require example.com/a v1.2.3 // indirect\n");
        assert!(errors.is_empty());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].verb.text, "require");
        assert_eq!(words(&found[0]), ["example.com/a", "v1.2.3"]);
        assert_eq!(found[0].comment.as_deref(), Some("indirect"));
        assert_eq!(found[0].line, 1);
    }

    #[test]
    fn a_block_gives_one_statement_per_line() {
        let source = "require (\n\ta v1 // indirect\n\n\t// note\n\tb v2\n)\nmodule m\n";
        let (found, errors) = directives(source);
        assert!(errors.is_empty());
        let shape: Vec<_> =
            found.iter().map(|d| (d.verb.text.as_str(), words(d), d.line)).collect();
        assert_eq!(
            shape,
            [
                ("require", vec!["a", "v1"], 2),
                ("require", vec!["b", "v2"], 5),
                ("module", vec!["m"], 7)
            ]
        );
        assert_eq!(found[0].comment.as_deref(), Some("indirect"));
        assert_eq!(found[1].comment, None);
    }

    #[test]
    fn quoted_strings_are_unquoted() {
        let (found, errors) =
            directives("module \"example.com/a b\"\nuse `./raw\\dir`\nx \"a\\\"b\\\\c\"\n");
        assert!(errors.is_empty());
        assert_eq!(words(&found[0]), ["example.com/a b"]);
        assert_eq!(words(&found[1]), ["./raw\\dir"]);
        assert_eq!(words(&found[2]), ["a\"b\\c"]);
        assert!(found[0].args[0].quoted);
    }

    #[test]
    fn punctuation_splits_words_without_spaces() {
        let (found, _) = directives("replace a=>b\nretract [v1,v2]\nreplace a v1=>../b\n");
        assert_eq!(words(&found[0]), ["a", "=>", "b"]);
        assert_eq!(words(&found[1]), ["[", "v1", ",", "v2", "]"]);
        assert_eq!(words(&found[2]), ["a", "v1", "=>", "../b"]);
    }

    #[test]
    fn an_equals_sign_inside_a_word_stays_in_it() {
        let (found, _) = directives("godebug default=go1.21\n");
        assert_eq!(words(&found[0]), ["default=go1.21"]);
    }

    #[test]
    fn comments_may_follow_a_slash_in_a_word() {
        let (found, _) = directives("require a/b v1//c\n");
        assert_eq!(words(&found[0]), ["a/b", "v1"]);
        assert_eq!(found[0].comment.as_deref(), Some("c"));
    }

    #[test]
    fn ranges_locate_tokens_in_the_source() {
        let source = "go 1.21\nrequire a v1\n";
        let (found, _) = directives(source);
        let range = found[1].range;
        assert_eq!(&source[range.start() as usize..range.end() as usize], "require a v1");
        let a = found[1].args[0].range;
        assert_eq!(&source[a.start() as usize..a.end() as usize], "a");
    }

    #[test]
    fn crlf_and_a_byte_order_mark_are_accepted() {
        let source = "\u{feff}module a\r\nrequire (\r\n\tb v1\r\n)\r\n";
        let (found, errors) = directives(source);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(found.len(), 2);
        let range = found[1].args[0].range;
        assert_eq!(&source[range.start() as usize..range.end() as usize], "b");
    }

    #[test]
    fn unclosed_strings_and_blocks_are_errors() {
        let (found, errors) = directives("module \"a\nrequire (\n\tb v1\n");
        assert_eq!(found.len(), 1, "the block line survives");
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].line, 1);
        assert!(errors[0].message.contains("never closed"));
        assert!(errors[1].message.contains("block"));
    }

    #[test]
    fn stray_punctuation_is_an_error() {
        let (found, errors) = directives(")\n(\nrequire (\n\t( x\n)\n");
        assert!(found.is_empty());
        assert_eq!(errors.len(), 3);
    }

    #[test]
    fn non_ascii_text_does_not_split_a_character() {
        let (found, errors) = directives("module héllo/ü // ünï\nrequire \"ö\" v1\n");
        assert!(errors.is_empty());
        assert_eq!(words(&found[0]), ["héllo/ü"]);
        assert_eq!(found[0].comment.as_deref(), Some("ünï"));
        assert_eq!(words(&found[1]), ["ö", "v1"]);
    }

    #[test]
    fn nothing_panics_on_odd_input() {
        for source in
            ["", "\n", "(", ")", "\"", "`", "\\", "a \"b\\", "=>", "//", "a//", "\u{feff}", "x (\n"]
        {
            let _ = directives(source);
        }
    }
}
