//! Locations within a source file: byte ranges, values located by them, and
//! the line index that finds the line an offset is on.

use std::{fmt, iter, ops::Range};

/// A range of bytes in a source file, from `start` up to but not including
/// `end`.
///
/// Ranges order by their start, then their end, which is source order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ByteRange {
    /// The byte offset of the start of the range.
    start: u32,

    /// The byte offset just past the end of the range.
    end: u32,
}

impl ByteRange {
    /// Create a range from `start` up to but not including `end`.
    pub fn new(start: u32, end: u32) -> Self {
        debug_assert!(start <= end, "byte range starts at {start}, after its end {end}");
        Self { start, end }
    }

    /// The byte offset of the start of the range.
    pub fn start(self) -> u32 {
        self.start
    }

    /// The byte offset just past the end of the range.
    pub fn end(self) -> u32 {
        self.end
    }

    /// The number of bytes in the range.
    pub fn len(self) -> u32 {
        self.end - self.start
    }

    /// Whether the range holds no bytes.
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Whether the byte at `offset` is in the range.
    pub fn contains(self, offset: u32) -> bool {
        self.start <= offset && offset < self.end
    }
}

impl From<ByteRange> for Range<usize> {
    fn from(range: ByteRange) -> Self {
        range.start as usize..range.end as usize
    }
}

/// A range of `usize` offsets, as parsers such as `toml` report them. The
/// offsets must fit in a `u32`, so the source must be shorter than 4 GiB.
impl From<Range<usize>> for ByteRange {
    fn from(range: Range<usize>) -> Self {
        let offset = |offset: usize| u32::try_from(offset).expect("a source is shorter than 4 GiB");
        Self::new(offset(range.start), offset(range.end))
    }
}

impl fmt::Display for ByteRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// A value with the byte range of the source that it was found at, such as
/// an import or a comment.
///
/// Located values order by their range first, so sorting them puts them in
/// source order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Located<T> {
    /// Where the value is in the source.
    pub range: ByteRange,

    /// The value.
    pub value: T,
}

impl<T> Located<T> {
    /// A value found at `range`.
    pub fn new(range: ByteRange, value: T) -> Self {
        Self { range, value }
    }
}

/// The start of every line of a text, for finding the 1-based line that a
/// byte offset is on, and the bytes of a line.
///
/// Lines end at `\n`, as in `grep -n` and GitHub, so the `\r` of a `\r\n`
/// line ending is the last character of its line. A lone `\r`, or U+2028 and
/// U+2029, which JavaScript also treats as line breaks, do not end a line.
///
/// ```
/// # use zenith_utils::location::{ByteRange, LineIndex};
/// let lines = LineIndex::new("import a from 'a';\nimport b from 'b';\n");
/// assert_eq!(lines.line(25), 2);
/// assert_eq!(lines.line_range(2), ByteRange::new(19, 38));
/// ```
///
/// Offsets are `u32`, like the spans of the JS/TS parser, so the text must be
/// shorter than 4 GiB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    /// The byte offset at which each line starts, in ascending order. The
    /// first line always starts at 0.
    starts: Vec<u32>,

    /// The length of the text.
    len: u32,
}

impl LineIndex {
    /// Build the line index of `text`.
    pub fn new(text: &str) -> Self {
        let breaks = text.match_indices('\n').map(|(offset, _)| offset as u32 + 1);
        Self { starts: iter::once(0).chain(breaks).collect(), len: text.len() as u32 }
    }

    /// The 1-based line containing the byte `offset`. An offset past the end
    /// of the text is on the last line.
    pub fn line(&self, offset: u32) -> u32 {
        // The number of lines that start at or before `offset`, which is at
        // least one since the first line starts at 0.
        self.starts.partition_point(|&start| start <= offset) as u32
    }

    /// The bytes of the 1-based `line`, including the `\n` that ends it. A
    /// line past the last is the empty range at the end of the text.
    pub fn line_range(&self, line: u32) -> ByteRange {
        let start = |line: u32| {
            self.starts.get(line.saturating_sub(1) as usize).copied().unwrap_or(self.len)
        };
        ByteRange::new(start(line), start(line + 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_exclude_their_end() {
        let range = ByteRange::new(4, 10);
        assert_eq!((range.start(), range.end(), range.len()), (4, 10, 6));
        assert!(!range.is_empty());
        assert!(ByteRange::new(3, 3).is_empty());
        assert!(range.contains(4) && range.contains(9));
        assert!(!range.contains(3) && !range.contains(10));
        assert!(!ByteRange::new(3, 3).contains(3));
        assert_eq!(ByteRange::default(), ByteRange::new(0, 0));

        let source = "import x from './x'";
        assert_eq!(&source[Range::from(ByteRange::new(14, 19))], "'./x'");
        assert_eq!(ByteRange::from(14..19), ByteRange::new(14, 19));
        assert_eq!(range.to_string(), "4..10");
    }

    #[test]
    fn finds_the_line_of_an_offset() {
        let lines = LineIndex::new("ab\r\ncd\n\ne");
        let found: Vec<_> = (0..=10).map(|offset| lines.line(offset)).collect();
        // The `\r\n` ends the first line, and offsets past the end are on the
        // last line.
        assert_eq!(found, [1, 1, 1, 1, 2, 2, 2, 3, 4, 4, 4]);
        assert_eq!(LineIndex::new("").line(0), 1);
    }

    #[test]
    fn line_ranges_include_the_line_ending() {
        let text = "ab\r\ncd\n";
        let lines = LineIndex::new(text);
        let found: Vec<_> =
            (1..=4).map(|line| &text[Range::from(lines.line_range(line))]).collect();
        // A trailing `\n` starts an empty last line, and a line past the last
        // is empty too.
        assert_eq!(found, ["ab\r\n", "cd\n", "", ""]);
        assert_eq!(lines.line_range(4), ByteRange::new(7, 7));
    }

    #[test]
    fn located_values_sort_into_source_order() {
        let mut found = [
            Located::new(ByteRange::new(9, 12), "b"),
            Located::new(ByteRange::new(0, 4), "z"),
            Located::new(ByteRange::new(5, 8), "a"),
        ];
        found.sort();
        assert_eq!(found.map(|located| located.value), ["z", "a", "b"]);
    }

    #[test]
    fn ranges_order_by_start_then_end() {
        let mut ranges = [ByteRange::new(5, 9), ByteRange::new(1, 8), ByteRange::new(5, 6)];
        ranges.sort();
        assert_eq!(ranges, [ByteRange::new(1, 8), ByteRange::new(5, 6), ByteRange::new(5, 9)]);
    }
}
