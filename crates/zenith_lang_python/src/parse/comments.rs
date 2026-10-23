//! The comments of a Python source, as the suppression logic needs them.

use std::ops::Range;

use zenith_lang::{Comment, CommentKind};
use zenith_utils::location::ByteRange;

/// The [`Comment`] of each of `spans`, the `#` comments of `source` in source
/// order.
pub(super) fn comments(source: &str, spans: &[Range<usize>]) -> Vec<Comment> {
    (0..spans.len()).map(|index| comment(source, spans, index)).collect()
}

/// The comment at `spans[index]`.
fn comment(source: &str, spans: &[Range<usize>], index: usize) -> Comment {
    // 1. A comment runs from its `#` to the end of its line, without the line
    //    ending.
    let Range { start, end } = spans[index].clone();
    let end = start + source[start..end].trim_end_matches(['\r', '\n']).len();

    // 2. Code before the comment on its line makes it trailing.
    let line_start = source[..start].rfind('\n').map_or(0, |newline| newline + 1);
    let trailing = !source[line_start..start].trim().is_empty();

    // 3. The next token starts after the whitespace and comments that follow.
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
        content: ByteRange::new((start + 1) as u32, end as u32),
        kind: CommentKind::Line,
        trailing,
        followed_by_newline: true,
        attached_to: attached_to as u32,
    }
}
