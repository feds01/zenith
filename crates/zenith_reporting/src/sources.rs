//! The source files that reports show excerpts of, read when first needed.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use zenith_diagnostics::{SourceId, SourceMap};
use zenith_utils::{fs::read_text, location::LineIndex};

/// The source files of the project at a root, read and indexed by line when
/// first asked about, in a [`SourceMap`] that reports can point into. A file
/// that can't be read is asked about only once.
pub struct Sources {
    /// The canonical project root.
    root: PathBuf,

    /// The sources read so far.
    map: SourceMap,

    /// The id and line index of each source read so far, by name: the POSIX
    /// path of a project file relative to the root, or the name a source was
    /// added under. `None` for a file that can't be read.
    files: HashMap<String, Option<(SourceId, LineIndex)>>,
}

impl Sources {
    /// The source files of the project at `root`.
    pub fn new(root: &Path) -> Self {
        Self { root: root.to_path_buf(), map: SourceMap::new(), files: HashMap::new() }
    }

    /// The sources read so far, which the ids point into.
    pub fn map(&self) -> &SourceMap {
        &self.map
    }

    /// Add a source that isn't read from the project, such as the
    /// configuration file, under the `name` that reports show.
    pub fn add(&mut self, name: &str, contents: &str) -> SourceId {
        let lines = LineIndex::new(contents);
        let id = self.map.add(name, contents);
        self.files.insert(name.to_string(), Some((id, lines)));
        id
    }

    /// The id of `file`, a POSIX path relative to the root, or `None` if it
    /// can't be read.
    pub fn source(&mut self, file: &str) -> Option<SourceId> {
        self.file(file).map(|(id, _)| *id)
    }

    /// The text of `file`, or `None` if it can't be read.
    pub fn text(&mut self, file: &str) -> Option<&str> {
        let id = self.source(file)?;
        Some(&self.map.get(id).contents)
    }

    /// The 1-based line and column at the byte `offset` of `file`, or `None`
    /// if it can't be read. Columns count characters, not bytes.
    pub fn position(&mut self, file: &str, offset: u32) -> Option<(u32, u32)> {
        let (id, lines) = self.file(file)?;
        let line = lines.line(offset);
        let (id, start) = (*id, lines.line_range(line).start() as usize);
        let text = &self.map.get(id).contents;
        let column = text.get(start..offset as usize)?.chars().count() as u32 + 1;
        Some((line, column))
    }

    /// The id and line index of `file`, read and indexed if it hasn't been
    /// yet.
    fn file(&mut self, file: &str) -> Option<&(SourceId, LineIndex)> {
        let Self { root, map, files } = self;
        files
            .entry(file.to_string())
            .or_insert_with(|| {
                let text = read_text(&root.join(file)).ok()?;
                let lines = LineIndex::new(&text);
                Some((map.add(file, text), lines))
            })
            .as_ref()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn reads_files_once_and_finds_lines_and_columns_in_characters() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("web")).unwrap();
        let text = "// é😀\nconst 日本 = 1; import('./b');\n";
        fs::write(dir.path().join("web/a.ts"), text).unwrap();

        let mut sources = Sources::new(dir.path());
        let offset = text.find("import").unwrap() as u32;
        assert_eq!(sources.position("web/a.ts", offset), Some((2, 15)));
        assert_eq!(sources.position("web/a.ts", 0), Some((1, 1)));
        assert_eq!(sources.text("web/a.ts"), Some(text));
        let id = sources.source("web/a.ts").unwrap();
        assert_eq!(sources.source("web/a.ts"), Some(id));
        assert_eq!(sources.map().get(id).path, "web/a.ts");
        assert_eq!(sources.position("web/missing.ts", 0), None);
    }

    #[test]
    fn sources_can_be_added_under_a_name() {
        let mut sources = Sources::new(Path::new("/nowhere"));
        let id = sources.add("zenith.toml", "[files]\nexclude = []\n");
        assert_eq!(sources.source("zenith.toml"), Some(id));
        assert_eq!(sources.position("zenith.toml", 8), Some((2, 1)));
    }
}
