//! Literate Haskell, as section 10.4 of the Report gives it: the code of a
//! literate file is in the lines that start with `>`, its Bird tracks, and in
//! the lines between a `\begin{code}` and an `\end{code}`, and the rest of it
//! is text, but for the lines of the C preprocessor, which stay, as GHC's
//! `unlit` leaves them for `cpp`.

use std::iter;

/// Write the code of `source`, a literate file, into `code`, as plain
/// Haskell. The text, and the lines of `\begin{code}` and `\end{code}`, are
/// made blank, and each Bird track a space, so that every offset and column of
/// the code stays where it was.
pub(super) fn unlit(source: &str, code: &mut String) {
    code.clear();
    code.reserve(source.len());
    let mut in_block = false;
    for line in source.split_inclusive('\n') {
        let delimiter = if in_block { "\\end{code}" } else { "\\begin{code}" };
        if line.starts_with(delimiter) {
            in_block = !in_block;
            blank(line, code);
        } else if in_block || line.starts_with('#') {
            code.push_str(line);
        } else if let Some(rest) = line.strip_prefix('>') {
            code.push(' ');
            code.push_str(rest);
        } else {
            blank(line, code);
        }
    }
}

/// Write `line` into `code` as spaces, as many bytes of them as each of its
/// characters takes, but for its line ending.
fn blank(line: &str, code: &mut String) {
    for c in line.chars() {
        match c {
            '\n' | '\r' => code.push(c),
            _ => code.extend(iter::repeat_n(' ', c.len_utf8())),
        }
    }
}
