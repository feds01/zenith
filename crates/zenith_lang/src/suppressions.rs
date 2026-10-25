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
//! Other comments next to it don't count as code. A language whose imports
//! sit inside statements that span lines, such as Terraform's blocks, extends
//! the target line to the end of each such statement that starts on it, with
//! [`Suppression::cover`].

use std::ops::Range;

use thin_vec::ThinVec;
use zenith_utils::location::{ByteRange, LineIndex, Located};

use crate::Comment;

/// The word that starts a suppression comment.
const MARKER: &str = "zenith-ignore";

/// A `zenith-ignore` comment, which removes the imports that start on its
/// target line from the contracts it names. It is located by the comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suppression {
    /// The ids of the contracts it suppresses, as written. Empty if it names
    /// none.
    pub contracts: Vec<String>,

    /// The reason given after the `:`, if any.
    pub reason: Option<String>,

    /// The target line, including its line ending. The suppression applies to
    /// the imports that start in it. Past the last line, it is the empty
    /// range at the end of the file. It reaches past the line to the end of
    /// the statements it [covers](Self::cover).
    pub target: ByteRange,
}

impl Suppression {
    /// The suppressions among `comments`, the comments of `source` in source
    /// order, located by their comments.
    pub fn collect(
        source: &str,
        comments: impl IntoIterator<Item = Comment>,
    ) -> ThinVec<Located<Suppression>> {
        // Most files have no suppressions, so index the lines only once one
        // is found.
        let mut lines = None;
        comments
            .into_iter()
            .filter_map(|comment| {
                let (contracts, reason) = parse_comment(source, &comment)?;
                let lines = lines.get_or_insert_with(|| LineIndex::new(source));
                let suppression =
                    Suppression { contracts, reason, target: target_line(lines, &comment) };
                Some(Located::new(comment.range, suppression))
            })
            .collect()
    }
}

impl Suppression {
    /// Extend the target to the end of `statement`, a statement with imports
    /// in it, if the statement starts in the target, so that a comment above
    /// a statement that spans lines applies to every import in it.
    pub fn cover(&mut self, statement: ByteRange) {
        if self.target.contains(statement.start()) && statement.end() > self.target.end() {
            self.target = ByteRange::new(self.target.start(), statement.end());
        }
    }
}

/// The target line of `comment`, in a source whose line index is `lines`.
///
/// The parser records whether a comment trails code, and where the token
/// after it starts, so other comments next to it are never mistaken for code.
fn target_line(lines: &LineIndex, comment: &Comment) -> ByteRange {
    let first_line = lines.line(comment.range.start());
    let last_line = lines.line(comment.range.end().saturating_sub(1));
    if comment.trailing {
        // After code on its first line.
        lines.line_range(first_line)
    } else if !comment.followed_by_newline && lines.line(comment.attached_to) == last_line {
        // Before code on its last line.
        lines.line_range(last_line)
    } else {
        // On a line, or lines, of its own.
        lines.line_range(last_line + 1)
    }
}

/// The contracts and reason of `comment`, if it is a suppression.
fn parse_comment(source: &str, comment: &Comment) -> Option<(Vec<String>, Option<String>)> {
    let content = &source[Range::from(comment.content)];
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

/// The `zenith-ignore` comments of a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSuppressions {
    /// The file, as a POSIX path relative to the root.
    pub file: String,

    /// Its suppressions, in source order. There is at least one.
    pub suppressions: ThinVec<Located<Suppression>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommentKind;

    /// The comment that `source` starts with, as a parser would report it,
    /// with the token after it, if any, starting at `attached_to`.
    fn first_comment(source: &str) -> Comment {
        let (kind, end, delimiter) = match source.starts_with("//") {
            true => (CommentKind::Line, source.find('\n').unwrap_or(source.len()), 0),
            false => (CommentKind::Block, source.find("*/").unwrap() + 2, 2),
        };
        let rest = &source[end..];
        let attached_to = end + rest.len() - rest.trim_start().len();
        Comment {
            range: ByteRange::new(0, end as u32),
            content: ByteRange::new(2, (end - delimiter) as u32),
            kind,
            trailing: false,
            followed_by_newline: rest.trim_start_matches([' ', '\t']).starts_with('\n'),
            attached_to: attached_to as u32,
        }
    }

    /// The suppressions in `source`, which starts with a comment, as
    /// `(contracts, reason, target line)`.
    fn suppressions(source: &str) -> Vec<(Vec<String>, Option<String>, u32)> {
        let lines = LineIndex::new(source);
        Suppression::collect(source, [first_comment(source)])
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
    fn the_target_line_follows_where_the_comment_sits() {
        let source = "import b from 'b'; /* zenith-ignore a */\nimport c from 'c';\n";
        let lines = LineIndex::new(source);
        let start = source.find("/*").unwrap() as u32;
        let end = start + "/* zenith-ignore a */".len() as u32;
        let comment = Comment {
            range: ByteRange::new(start, end),
            content: ByteRange::new(start + 2, end - 2),
            kind: CommentKind::Block,
            trailing: true,
            followed_by_newline: true,
            attached_to: source.rfind("import").unwrap() as u32,
        };
        let target = |comment: Comment| target_line(&lines, &comment);
        // After code, it is the comment's own line.
        assert_eq!(target(comment), lines.line_range(1));
        // Before code on its last line, it is that line.
        let leading =
            Comment { trailing: false, followed_by_newline: false, attached_to: end, ..comment };
        assert_eq!(target(leading), lines.line_range(1));
        // On a line of its own, it is the next line.
        let alone = Comment { trailing: false, ..comment };
        assert_eq!(target(alone), lines.line_range(2));
    }

    #[test]
    fn covers_the_statements_that_start_on_the_target_line() {
        let suppression = |start, end| Suppression {
            contracts: vec!["a".to_owned()],
            reason: None,
            target: ByteRange::new(start, end),
        };
        let mut covering = suppression(10, 20);
        covering.cover(ByteRange::new(12, 50));
        assert_eq!(covering.target, ByteRange::new(10, 50));
        // A statement that starts elsewhere, or ends inside the target, changes
        // nothing.
        for statement in [ByteRange::new(20, 60), ByteRange::new(5, 60), ByteRange::new(12, 15)] {
            let mut unchanged = suppression(10, 20);
            unchanged.cover(statement);
            assert_eq!(unchanged.target, ByteRange::new(10, 20), "{statement:?}");
        }
    }
}
