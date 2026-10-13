//! Helpers for the text of files that zenith reads.

use std::borrow::Cow;

/// The markers that git writes around the sides of a merge conflict: the
/// start of ours, the common ancestor's (in the `diff3` style), the start of
/// theirs, and the end.
const CONFLICT_MARKERS: [&str; 4] = ["<<<<<<<", "|||||||", "=======", ">>>>>>>"];

/// `text` with each line that is one of git's merge conflict markers turned
/// into a line comment of the same length, by replacing its first character
/// with `comment`, e.g. `#` for TOML. Every line of every side is kept, at the
/// same byte offsets, so positions in the result are positions in `text`.
/// Borrowed if there are no markers.
pub fn comment_out_conflict_markers(text: &str, comment: char) -> Cow<'_, str> {
    if !text.lines().any(is_conflict_marker) {
        return Cow::Borrowed(text);
    }
    let mut commented = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        match is_conflict_marker(line.trim_end_matches('\n')) {
            true => {
                commented.push(comment);
                commented.push_str(&line[1..]);
            }
            false => commented.push_str(line),
        }
    }
    Cow::Owned(commented)
}

/// Whether `line` is one of git's merge conflict markers: seven `<`, `|`, `=`
/// or `>`, then the end of the line, or a space and a label.
fn is_conflict_marker(line: &str) -> bool {
    CONFLICT_MARKERS.iter().any(|marker| {
        line.strip_prefix(marker)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\r']))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_whole_marker_lines_are_commented_out() {
        let commented = |text| comment_out_conflict_markers(text, '#');
        assert_eq!(commented("a\n=======\nb\n"), "a\n#======\nb\n");
        assert_eq!(commented(">>>>>>> theirs\r\n"), "#>>>>>> theirs\r\n");
        assert_eq!(commented("<<<<<<< HEAD\nx\n||||||| base\n"), "#<<<<<< HEAD\nx\n#|||||| base\n");
        assert!(matches!(commented("========\n"), Cow::Borrowed(_)));
        assert!(matches!(commented("  <<<<<<< HEAD\n"), Cow::Borrowed(_)));
        assert!(matches!(commented("no markers\n"), Cow::Borrowed(_)));
    }
}
