//! Locations within a source file.

use std::{fmt, ops::Range};

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
}

impl From<ByteRange> for Range<usize> {
    fn from(range: ByteRange) -> Self {
        range.start as usize..range.end as usize
    }
}

impl fmt::Display for ByteRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
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
        assert_eq!(ByteRange::default(), ByteRange::new(0, 0));

        let source = "import x from './x'";
        assert_eq!(&source[Range::from(ByteRange::new(14, 19))], "'./x'");
        assert_eq!(range.to_string(), "4..10");
    }

    #[test]
    fn ranges_order_by_start_then_end() {
        let mut ranges = [ByteRange::new(5, 9), ByteRange::new(1, 8), ByteRange::new(5, 6)];
        ranges.sort();
        assert_eq!(ranges, [ByteRange::new(1, 8), ByteRange::new(5, 6), ByteRange::new(5, 9)]);
    }
}
