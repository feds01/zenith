//! Textual utilities: mapping byte offsets to line and column numbers.

/// A table of line start offsets for a source file, playing the role of
/// `bracketlint`'s `LineRanges`. Lines and columns are 1-based; columns count
/// characters, not bytes.
#[derive(Debug, Clone)]
pub struct LineIndex {
    /// The byte offset at which each line starts.
    starts: Vec<u32>,
}

impl LineIndex {
    /// Build a line index for the given source text.
    pub fn new(contents: &str) -> Self {
        let mut starts = Vec::with_capacity(contents.len() / 32 + 1);
        starts.push(0);
        for (offset, byte) in contents.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(offset as u32 + 1);
            }
        }
        Self { starts }
    }

    /// The 1-based line number containing the byte `offset`.
    pub fn line(&self, offset: u32) -> u32 {
        match self.starts.binary_search(&offset) {
            Ok(index) => index as u32 + 1,
            Err(index) => index as u32,
        }
    }

    /// The 1-based `(line, column)` for the byte `offset`.
    pub fn line_column(&self, contents: &str, offset: u32) -> (u32, u32) {
        let line = self.line(offset);
        let start = self.starts[line as usize - 1] as usize;
        let end = (offset as usize).min(contents.len());
        let column = contents.get(start..end).map_or(0, |s| s.chars().count()) as u32 + 1;
        (line, column)
    }

    /// The byte offset at which the 1-based `line` starts.
    pub fn line_start(&self, line: u32) -> Option<u32> {
        self.starts.get(line.checked_sub(1)? as usize).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_offsets_to_lines_and_columns() {
        let src = "import a from 'a';\nimport b from 'b';\n\nconst é = 1; import('c');\n";
        let index = LineIndex::new(src);
        assert_eq!(index.line_column(src, 0), (1, 1));
        assert_eq!(index.line_column(src, 19), (2, 1));
        assert_eq!(index.line_column(src, 26), (2, 8));
        let dynamic = src.find("import('c')").unwrap() as u32;
        assert_eq!(index.line_column(src, dynamic), (4, 14));
        assert_eq!(index.line_start(3), Some(38));
    }
}
