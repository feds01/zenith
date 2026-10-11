//! `zenith-ignore` suppression comments.
//!
//! ```ts
//! // zenith-ignore web-layers: legacy, tracked in ENG-123
//! import { x } from "@/domains/orders";
//!
//! import { y } from "@/app/types"; // zenith-ignore web-layers other-contract
//! ```
//!
//! A suppression is a line or block comment that starts with `zenith-ignore`,
//! then the ids of the contracts it suppresses, separated by spaces or commas,
//! then optionally a `:` and the reason. In a block comment, the leading `*`
//! of each line is ignored, so JSDoc-style comments work too.
//!
//! A comment applies to the imports that start on one line, its target line:
//!
//! - after code, the comment's own line (a trailing comment);
//! - before code, the line where the comment ends;
//! - on a line, or lines, of its own, the next line.
//!
//! Other comments next to it don't count as code.

use oxc_ast::Comment;
use thin_vec::ThinVec;
use zenith_utils::location::{ByteRange, LineIndex};

/// The word that starts a suppression comment.
const MARKER: &str = "zenith-ignore";

/// A `zenith-ignore` comment, which removes the imports that start on its
/// target line from the contracts it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suppression {
    /// The ids of the contracts it suppresses, as written. Empty if it names
    /// none.
    pub contracts: Vec<String>,

    /// The reason given after the `:`, if any.
    pub reason: Option<String>,

    /// Where the comment is in the file.
    pub range: ByteRange,

    /// The target line, including its line ending. The suppression applies to
    /// the imports that start in it. Past the last line, it is the empty
    /// range at the end of the file.
    pub target: ByteRange,
}

impl Suppression {
    /// The suppressions among `comments`, the comments of `source` in source
    /// order.
    pub(crate) fn collect(source: &str, comments: &[Comment]) -> ThinVec<Suppression> {
        // Most files have no suppressions, so index the lines only once one
        // is found.
        let mut lines = None;
        comments
            .iter()
            .filter_map(|comment| {
                let (contracts, reason) = parse_comment(source, comment)?;
                let lines = lines.get_or_insert_with(|| LineIndex::new(source));
                Some(Suppression {
                    contracts,
                    reason,
                    range: ByteRange::new(comment.span.start, comment.span.end),
                    target: target_line(source, lines, comments, comment),
                })
            })
            .collect()
    }
}

/// The target line of `comment`, one of `comments` in `source`, whose line
/// index is `lines`.
fn target_line(
    source: &str,
    lines: &LineIndex,
    comments: &[Comment],
    comment: &Comment,
) -> ByteRange {
    let first_line = lines.line(comment.span.start);
    let last_line = lines.line(comment.span.end.saturating_sub(1));
    let (first, last) = (lines.line_range(first_line), lines.line_range(last_line));
    if has_code(source, comments, first.start(), comment.span.start) {
        first
    } else if has_code(source, comments, comment.span.end, last.end()) {
        last
    } else {
        lines.line_range(last_line + 1)
    }
}

/// The contracts and reason of `comment`, if it is a suppression.
fn parse_comment(source: &str, comment: &Comment) -> Option<(Vec<String>, Option<String>)> {
    let content = comment.content_span().source_text(source);
    // Skip the `*`s that start a JSDoc comment (`/**`) or the lines of a block.
    let rest = content.trim_start_matches(|c: char| c == '*' || c.is_whitespace());
    let rest = rest.strip_prefix(MARKER)?;
    // The marker must be a word of its own, unlike `zenith-ignored`.
    if !(rest.is_empty() || rest.starts_with(|c: char| c.is_whitespace() || c == ':')) {
        return None;
    }

    let text = join_lines(rest);
    let (ids, reason) = match text.split_once(':') {
        Some((ids, reason)) => (ids, Some(reason.trim()).filter(|reason| !reason.is_empty())),
        None => (text.as_str(), None),
    };
    let contracts = ids
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect();
    Some((contracts, reason.map(str::to_owned)))
}

/// The lines of a comment's text joined with spaces, without the leading `*`
/// of each line of a block comment.
fn join_lines(text: &str) -> String {
    let mut joined = String::with_capacity(text.len());
    for line in text.lines() {
        let line = line.trim().trim_start_matches('*').trim_start();
        if !line.is_empty() {
            if !joined.is_empty() {
                joined.push(' ');
            }
            joined.push_str(line);
        }
    }
    joined
}

/// Whether the bytes `from..to` of `source` hold code: anything but
/// whitespace and `comments`, which are in source order.
fn has_code(source: &str, comments: &[Comment], from: u32, to: u32) -> bool {
    let is_code = |from: u32, to: u32| {
        source.get(from as usize..to as usize).is_some_and(|text| !text.trim().is_empty())
    };
    let first = comments.partition_point(|comment| comment.span.end <= from);
    let mut position = from;
    for comment in comments[first..].iter().take_while(|comment| comment.span.start < to) {
        if is_code(position, comment.span.start) {
            return true;
        }
        position = position.max(comment.span.end);
    }
    is_code(position, to)
}

#[cfg(test)]
mod tests {
    use std::{ops::Range, path::Path};

    use super::*;
    use crate::ParsedFile;

    /// The suppressions in `source`, as `(contracts, reason, target line)`.
    fn suppressions(source: &str) -> Vec<(Vec<String>, Option<String>, u32)> {
        let parsed = ParsedFile::parse_source(Path::new("x.ts"), source);
        assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
        let lines = LineIndex::new(source);
        parsed
            .suppressions
            .into_iter()
            .map(|suppression| {
                let target_line = lines.line(suppression.target.start());
                (suppression.contracts, suppression.reason, target_line)
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
    fn syntax() {
        for (comment, contracts, reason) in [
            ("// zenith-ignore web-layers", &["web-layers"][..], None),
            ("//zenith-ignore web-layers", &["web-layers"], None),
            ("// zenith-ignore a b", &["a", "b"], None),
            ("// zenith-ignore a, b,c", &["a", "b", "c"], None),
            ("// zenith-ignore a: legacy, see ENG-123", &["a"], Some("legacy, see ENG-123")),
            (
                "// zenith-ignore a b:reason: with a colon",
                &["a", "b"],
                Some("reason: with a colon"),
            ),
            ("// zenith-ignore a:", &["a"], None),
            ("// zenith-ignore", &[], None),
            ("// zenith-ignore: no contract", &[], Some("no contract")),
            ("/* zenith-ignore a: block */", &["a"], Some("block")),
            ("/** zenith-ignore a */", &["a"], None),
        ] {
            let source = format!("{comment}\nimport a from 'a';\n");
            assert_eq!(suppressions(&source), found(contracts, reason, 2), "{comment}");
        }
    }

    #[test]
    fn other_comments_are_not_suppressions() {
        for comment in [
            "// zenith-ignored a",
            "// zenith-ignore-next-line a",
            "// TODO: zenith-ignore a",
            "// eslint-disable-next-line",
            "/* zenith */",
            "//",
            "/**/",
        ] {
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
        let parsed = ParsedFile::parse_source(Path::new("x.ts"), source);
        let found: Vec<_> = parsed
            .suppressions
            .iter()
            .map(|suppression| {
                (&source[Range::from(suppression.range)], &source[Range::from(suppression.target)])
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
        let parsed = ParsedFile::parse_source(Path::new("x.ts"), source);
        let end = source.len() as u32;
        assert_eq!(parsed.suppressions[0].target, ByteRange::new(end, end));
    }
}
