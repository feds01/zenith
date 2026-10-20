//! Conversions from what oxc reports to the language-neutral types of
//! `zenith_lang`.

use std::borrow::Cow;

use oxc_ast::{Comment, CommentKind as OxcCommentKind};
use oxc_span::{LabeledSpan, Span};
use zenith_lang::{self as lang, SyntaxError};
use zenith_utils::location::{ByteRange, Located};

/// The comment that oxc found, as `zenith_lang` describes comments.
pub(super) fn comment(comment: &Comment) -> lang::Comment {
    let kind = match comment.kind {
        OxcCommentKind::Line | OxcCommentKind::HtmlOpen | OxcCommentKind::HtmlClose => {
            lang::CommentKind::Line
        }
        OxcCommentKind::SingleLineBlock | OxcCommentKind::MultiLineBlock => {
            lang::CommentKind::Block
        }
    };
    lang::Comment {
        range: byte_range(comment.span),
        content: byte_range(comment.content_span()),
        kind,
        trailing: comment.is_trailing(),
        followed_by_newline: comment.followed_by_newline(),
        attached_to: comment.attached_to,
    }
}

/// The error with oxc's `message`, at the code its primary label points at,
/// or its first label if none is primary.
pub(super) fn syntax_error(
    message: Cow<'static, str>,
    labels: &[LabeledSpan],
) -> Located<SyntaxError> {
    let label = labels.iter().find(|label| label.primary()).or_else(|| labels.first());
    let span = label.map_or(Span::default(), LabeledSpan::span);
    Located::new(byte_range(span), SyntaxError { message })
}

/// The range of `span`.
fn byte_range(span: Span) -> ByteRange {
    ByteRange::new(span.start, span.end)
}

#[cfg(test)]
mod tests {
    use std::{ops::Range, path::Path};

    use zenith_lang::Suppression;
    use zenith_utils::location::{ByteRange, LineIndex};

    use crate::Parser;

    /// The suppressions in `source`, as `(contracts, reason, target line)`.
    fn suppressions(source: &str) -> Vec<(Vec<String>, Option<String>, u32)> {
        let parsed = Parser::new().parse_source(Path::new("x.ts"), source);
        assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
        let lines = LineIndex::new(source);
        parsed
            .suppressions
            .into_iter()
            .map(|suppression| {
                let Suppression { contracts, reason, target } = suppression.value;
                (contracts, reason, lines.line(target.start()))
            })
            .collect()
    }

    fn found(
        contracts: &[&str],
        reason: Option<&str>,
        target_line: u32,
    ) -> Vec<(Vec<String>, Option<String>, u32)> {
        let contracts = contracts.iter().map(|id| id.to_string()).collect();
        vec![(contracts, reason.map(str::to_string), target_line)]
    }

    #[test]
    fn comments_have_their_suppressions() {
        for (comment, contracts, reason) in [
            ("// zenith-ignore web-layers", &["web-layers"][..], None),
            ("//zenith-ignore a, b,c", &["a", "b", "c"], None),
            ("// zenith-ignore a: legacy", &["a"], Some("legacy")),
            ("/* zenith-ignore a: block */", &["a"], Some("block")),
            ("/** zenith-ignore a */", &["a"], None),
        ] {
            let source = format!("{comment}\nimport a from 'a';\n");
            assert_eq!(suppressions(&source), found(contracts, reason, 2), "{comment}");
        }
    }

    #[test]
    fn other_comments_are_not_suppressions() {
        for comment in ["// zenith-ignored a", "// TODO: zenith-ignore a", "/* zenith */", "/**/"] {
            let source = format!("{comment}\nimport a from 'a';\n");
            assert_eq!(suppressions(&source), [], "{comment}");
        }
        // Nor is the marker in a string.
        assert_eq!(suppressions("const a = '// zenith-ignore a';\n"), []);
    }

    #[test]
    fn target_lines() {
        for (source, target_line) in [
            // A comment on a line of its own applies to the next line.
            ("// zenith-ignore a\nimport b from 'b';\n", 2),
            ("  /* zenith-ignore a */  \nimport b from 'b';\n", 2),
            ("import b from 'b';\n\n// zenith-ignore a\n", 4),
            // A trailing comment applies to its own line.
            ("import b from 'b'; // zenith-ignore a\n", 1),
            ("import b from 'b'; /* zenith-ignore a */\n", 1),
            ("\nimport {\n  b,\n} from 'b'; // zenith-ignore a\n", 4),
            // A comment before code applies to the line where it ends.
            ("/* zenith-ignore a */ import b from 'b';\n", 1),
            ("/* zenith-ignore a\n */ import b from 'b';\n", 2),
            // A comment over several lines applies to the line after its end.
            ("/*\n * zenith-ignore a\n */\nimport b from 'b';\n", 4),
            // Other comments are not code.
            ("/* zenith-ignore a */ // why\nimport b from 'b';\n", 2),
            ("/* note */ // zenith-ignore a\nimport b from 'b';\n", 2),
            ("/* one\n two */ /* zenith-ignore a */\nimport b from 'b';\n", 3),
            ("import b from 'b'; /* x */ // zenith-ignore a\n", 1),
            // Lines end at `\n`, whatever comes before it.
            ("// zenith-ignore a\r\nimport b from 'b';\r\n", 2),
            ("import b from 'b'; // zenith-ignore a\r\n", 1),
        ] {
            assert_eq!(suppressions(source), found(&["a"], None, target_line), "{source:?}");
        }
    }

    #[test]
    fn block_comments_join_their_lines() {
        let source = "/**
 * zenith-ignore a,
 *   b: legacy code that
 * we will move
 */
import c from 'c';
";
        assert_eq!(
            suppressions(source),
            found(&["a", "b"], Some("legacy code that we will move"), 6)
        );
    }

    #[test]
    fn records_the_comment_and_its_target_line() {
        let source = "import a from 'a'; // zenith-ignore a\nconst é = 1; /* zenith-ignore b */\n";
        let parsed = Parser::new().parse_source(Path::new("x.ts"), source);
        let found: Vec<_> = parsed
            .suppressions
            .iter()
            .map(|suppression| {
                let target = suppression.value.target;
                (&source[Range::from(suppression.range)], &source[Range::from(target)])
            })
            .collect();
        assert_eq!(
            found,
            [
                ("// zenith-ignore a", "import a from 'a'; // zenith-ignore a\n"),
                ("/* zenith-ignore b */", "const é = 1; /* zenith-ignore b */\n"),
            ]
        );
        // A comment on the last line, with nothing after it, applies to the
        // empty range at the end of the file.
        let source = "import a from 'a';\n// zenith-ignore a";
        let parsed = Parser::new().parse_source(Path::new("x.ts"), source);
        let end = source.len() as u32;
        assert_eq!(parsed.suppressions[0].value.target, ByteRange::new(end, end));
    }
}
