//! The comments of a Rust source, as the suppression logic needs them.

use std::ops::Range;

use zenith_lang::{Comment, CommentKind};
use zenith_utils::location::ByteRange;

/// The [`Comment`] of each of `spans`, the `//` and `/* */` comments of
/// `source` in source order.
pub(super) fn comments(source: &str, spans: &[Range<usize>]) -> Vec<Comment> {
    (0..spans.len()).map(|index| comment(source, spans, index)).collect()
}

/// The comment at `spans[index]`.
fn comment(source: &str, spans: &[Range<usize>], index: usize) -> Comment {
    let Range { start, end } = spans[index].clone();
    let text = &source[start..end];

    // 1. A line comment may include its line ending, which is not part of it.
    let (kind, content) = if text.starts_with("//") {
        let text = text.trim_end_matches(['\r', '\n']);
        (CommentKind::Line, start + 2..start + text.len())
    } else {
        let close = if text.len() >= 4 && text.ends_with("*/") { 2 } else { 0 };
        (CommentKind::Block, start + 2..end - close)
    };
    let end = if kind == CommentKind::Line { content.end } else { end };

    // 2. Code before the comment on its line makes it trailing. Comments before it
    //    on the line are not code.
    let line_start = source[..start].rfind('\n').map_or(0, |newline| newline + 1);
    let mut from = line_start;
    let mut trailing = false;
    let first = spans[..index].iter().rposition(|before| before.end <= line_start);
    for before in &spans[first.map_or(0, |first| first + 1)..index] {
        trailing |= before.start > from && !source[from..before.start].trim().is_empty();
        from = from.max(before.end);
    }
    trailing |= !source[from..start].trim().is_empty();

    // 3. A line break follows the comment if only blanks come before it, or the end
    //    of the file.
    let rest = source[end..].trim_start_matches([' ', '\t']);
    let followed_by_newline = rest.is_empty() || rest.starts_with(['\n', '\r']);

    // 4. The next token starts after the whitespace and comments that follow.
    let mut attached_to = end;
    for next in &spans[index + 1..] {
        let rest = &source[attached_to..];
        attached_to += rest.len() - rest.trim_start().len();
        if next.start != attached_to {
            break;
        }
        attached_to = next.end;
    }
    let rest = &source[attached_to..];
    attached_to += rest.len() - rest.trim_start().len();

    Comment {
        range: ByteRange::new(start as u32, end as u32),
        content: ByteRange::new(content.start as u32, content.end as u32),
        kind,
        trailing,
        followed_by_newline,
        attached_to: attached_to as u32,
    }
}
