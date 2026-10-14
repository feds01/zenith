//! The sources that reports point into, and where in them a span is.

use std::{
    collections::HashMap,
    fmt,
    path::{Path, PathBuf},
};

use zenith_utils::{
    fs::read_text,
    location::{ByteRange, LineIndex},
    path::{adjust_canonicalisation, relative_posix},
};

/// Identifies a source in a [`SourceMap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(u32);

/// A byte range of a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    /// The source that the range is in.
    pub source: SourceId,

    /// The byte range in the source.
    pub range: ByteRange,
}

impl Span {
    /// A span of `range` in `source`.
    pub fn new(source: SourceId, range: ByteRange) -> Self {
        Self { source, range }
    }
}

/// A 1-based line and column of a source. Columns count characters, not
/// bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    /// The line.
    pub line: u32,

    /// The column.
    pub column: u32,
}

/// Where a span starts: the path of its source, and the position in it.
/// Shown as `path:line:col`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location<'a> {
    /// The path of the source, as reports show it.
    pub path: &'a str,

    /// The position of the start of the span.
    pub position: Position,
}

impl fmt::Display for Location<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.path, self.position.line, self.position.column)
    }
}

/// A source file that reports can point into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The path of the file, as shown in reports.
    pub path: String,

    /// The contents of the file.
    pub contents: String,

    /// The start of each line of the contents.
    lines: LineIndex,
}

impl Source {
    /// The source of `contents`, shown as `path`.
    fn new(path: String, contents: String) -> Self {
        let lines = LineIndex::new(&contents);
        Self { path, contents, lines }
    }

    /// The 1-based line that the byte `offset` is on. An offset past the end
    /// is on the last line.
    pub fn line(&self, offset: u32) -> u32 {
        self.lines.line(offset)
    }

    /// The position of the byte `offset`. An offset past the end is just
    /// after the last character.
    pub fn position(&self, offset: u32) -> Position {
        let line = self.lines.line(offset);
        let start = self.lines.line_range(line).start();
        let before = self.contents[start as usize..]
            .char_indices()
            .take_while(|(index, _)| start as usize + index < offset as usize)
            .count();
        Position { line, column: before as u32 + 1 }
    }

    /// The whole lines that `range` is on: the 1-based number of the first,
    /// and their bytes, including the `\n` that ends the last. The end of a
    /// source that ends with a `\n` is on its last line, not the empty one
    /// after it, as excerpts show it.
    pub fn lines(&self, range: ByteRange) -> (u32, ByteRange) {
        let line = |offset: u32| match offset as usize == self.contents.len() {
            true if self.contents.ends_with('\n') => self.lines.line(offset - 1),
            _ => self.lines.line(offset),
        };
        let (first, last) = (line(range.start()), line(range.end()));
        let lines =
            ByteRange::new(self.lines.line_range(first).start(), self.lines.line_range(last).end());
        (first, lines)
    }
}

/// The sources that reports point into, each with a [`SourceId`].
///
/// A map with a root reads the files under it when they are first asked for,
/// by their POSIX path relative to the root, and shows them by that path.
/// Other sources, such as the configuration file, are added with their
/// contents.
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    /// The canonical directory that files are read from, if any.
    root: Option<PathBuf>,

    /// The sources, indexed by their id.
    sources: Vec<Source>,

    /// The id of each source, by the path it is shown as. `None` for a file
    /// under the root that can't be read, so that it is only tried once.
    ids: HashMap<String, Option<SourceId>>,
}

impl SourceMap {
    /// An empty source map, of sources added with their contents.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty source map that reads the files under `root`, a canonical
    /// path, when they are first asked for.
    pub fn with_root(root: &Path) -> Self {
        Self { root: Some(root.to_path_buf()), ..Self::default() }
    }

    /// Add a source, shown as `path`, and return its id.
    pub fn add(&mut self, path: impl Into<String>, contents: impl Into<String>) -> SourceId {
        let path = path.into();
        let id = SourceId(u32::try_from(self.sources.len()).expect("fewer than 2^32 sources"));
        self.ids.insert(path.clone(), Some(id));
        self.sources.push(Source::new(path, contents.into()));
        id
    }

    /// Add the file at `path`, with the `contents` it was read with, and
    /// return its id. It is shown by its POSIX path relative to the root if
    /// it is inside it, and as `path` otherwise.
    pub fn add_file(&mut self, path: &Path, contents: impl Into<String>) -> SourceId {
        let relative = self
            .root
            .as_deref()
            .and_then(|root| relative_posix(root, &adjust_canonicalisation(path)));
        let shown = relative.unwrap_or_else(|| path.display().to_string());
        self.add(shown, contents)
    }

    /// The id of the source shown as `path`. A file under the root, named by
    /// its POSIX path relative to it, is read if it hasn't been yet. `None`
    /// if it can't be read, or if there is no such source.
    pub fn file(&mut self, path: &str) -> Option<SourceId> {
        if let Some(id) = self.ids.get(path) {
            return *id;
        }
        let root = self.root.as_ref()?;
        match read_text(&root.join(path)) {
            Ok(contents) => Some(self.add(path, contents)),
            Err(_) => {
                self.ids.insert(path.to_string(), None);
                None
            }
        }
    }

    /// The source with the given id, which must come from this map.
    pub fn get(&self, id: SourceId) -> &Source {
        &self.sources[id.0 as usize]
    }

    /// Where `span` starts, which must be in this map.
    pub fn location(&self, span: Span) -> Location<'_> {
        let source = self.get(span.source);
        Location { path: &source.path, position: source.position(span.range.start()) }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn gives_each_source_its_own_id() {
        let mut sources = SourceMap::new();
        let config = sources.add("zenith.toml", "[files]\n");
        let module = sources.add("web/src/a.ts", "import './b';\n");

        assert_ne!(config, module);
        assert_eq!(sources.get(config).path, "zenith.toml");
        assert_eq!(sources.get(module).contents, "import './b';\n");
        assert_eq!(sources.file("zenith.toml"), Some(config));
    }

    #[test]
    fn finds_positions_in_characters() {
        let text = "// é😀\nconst 日本 = 1; import('./b');\n";
        let mut sources = SourceMap::new();
        let id = sources.add("web/a.ts", text);
        let source = sources.get(id);

        let offset = text.find("import").unwrap() as u32;
        assert_eq!(source.position(offset), Position { line: 2, column: 15 });
        assert_eq!(source.position(0), Position { line: 1, column: 1 });
        assert_eq!(source.line(offset), 2);
        let span = Span::new(id, ByteRange::new(offset, offset + 6));
        assert_eq!(sources.location(span).to_string(), "web/a.ts:2:15");
    }

    #[test]
    fn reads_files_under_the_root_once() {
        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        fs::create_dir_all(root.join("web")).unwrap();
        fs::write(root.join("web/a.ts"), "import './b';\n").unwrap();

        let mut sources = SourceMap::with_root(&root);
        let id = sources.file("web/a.ts").unwrap();
        assert_eq!(sources.file("web/a.ts"), Some(id));
        assert_eq!(sources.get(id).path, "web/a.ts");
        assert_eq!(sources.file("web/b.ts"), None);
        // A file that couldn't be read isn't tried again.
        fs::write(root.join("web/b.ts"), "").unwrap();
        assert_eq!(sources.file("web/b.ts"), None);
        assert_eq!(SourceMap::new().file("web/a.ts"), None);
    }

    #[test]
    fn shows_added_files_relative_to_the_root() {
        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        fs::create_dir_all(root.join("project")).unwrap();
        let inside = root.join("project/zenith.toml");
        fs::write(&inside, "").unwrap();
        let outside = root.join("zenith.toml");
        fs::write(&outside, "").unwrap();

        let mut sources = SourceMap::with_root(&root.join("project"));
        let id = sources.add_file(&inside, "");
        assert_eq!(sources.get(id).path, "zenith.toml");
        let id = sources.add_file(&outside, "");
        assert_eq!(sources.get(id).path, outside.display().to_string());
    }
}
