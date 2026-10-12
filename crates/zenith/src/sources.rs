//! The source files of a project, read when first needed, for showing where
//! imports are.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use zenith_utils::{fs::read_text, location::LineIndex};

/// A source file's text and the start of each of its lines.
struct SourceFile {
    /// The text.
    text: String,

    /// The lines of the text.
    lines: LineIndex,
}

/// The source files of the project at a root, read and indexed by line when
/// first asked about. A file that can't be read is asked about only once.
pub(crate) struct SourceFiles {
    /// The canonical project root.
    root: PathBuf,

    /// The files read so far, by POSIX path relative to the root.
    files: HashMap<String, Option<SourceFile>>,
}

impl SourceFiles {
    /// The source files of the project at `root`.
    pub(crate) fn new(root: &Path) -> Self {
        Self { root: root.to_path_buf(), files: HashMap::new() }
    }

    /// The text of `file`, a POSIX path relative to the root, or `None` if it
    /// can't be read.
    pub(crate) fn text(&mut self, file: &str) -> Option<&str> {
        self.file(file).map(|file| file.text.as_str())
    }

    /// The 1-based line and column at the byte `offset` of `file`, or `None`
    /// if the file can't be read. Columns count characters, not bytes.
    pub(crate) fn position(&mut self, file: &str, offset: u32) -> Option<(u32, u32)> {
        let SourceFile { text, lines } = self.file(file)?;
        let line = lines.line(offset);
        let start = lines.line_range(line).start() as usize;
        let column = text.get(start..offset as usize)?.chars().count() as u32 + 1;
        Some((line, column))
    }

    /// The file, read and indexed if it hasn't been yet.
    fn file(&mut self, file: &str) -> Option<&SourceFile> {
        let root = &self.root;
        self.files
            .entry(file.to_string())
            .or_insert_with(|| {
                let text = read_text(&root.join(file)).ok()?;
                Some(SourceFile { lines: LineIndex::new(&text), text })
            })
            .as_ref()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn finds_lines_and_columns_in_characters() {
        let dir = std::env::temp_dir().join(format!("zenith-sources-{}", std::process::id()));
        fs::create_dir_all(dir.join("web")).unwrap();
        let text = "// é😀\nconst 日本 = 1; import('./b');\n";
        fs::write(dir.join("web/a.ts"), text).unwrap();

        let mut sources = SourceFiles::new(&dir);
        let offset = text.find("import").unwrap() as u32;
        assert_eq!(sources.position("web/a.ts", offset), Some((2, 15)));
        assert_eq!(sources.position("web/a.ts", 0), Some((1, 1)));
        assert_eq!(sources.text("web/a.ts"), Some(text));
        assert_eq!(sources.position("web/missing.ts", 0), None);
        fs::remove_dir_all(dir).unwrap();
    }
}
