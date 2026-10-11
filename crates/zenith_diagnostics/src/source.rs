//! The sources that reports point into.

use zenith_utils::location::ByteRange;

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

/// A source file that reports can point into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The path of the file, as shown in reports.
    pub path: String,

    /// The contents of the file.
    pub contents: String,
}

/// The sources that reports point into, each with a [`SourceId`].
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    /// The sources, indexed by their id.
    sources: Vec<Source>,
}

impl SourceMap {
    /// An empty source map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a source, and return its id.
    pub fn add(&mut self, path: impl Into<String>, contents: impl Into<String>) -> SourceId {
        let id = SourceId(u32::try_from(self.sources.len()).expect("fewer than 2^32 sources"));
        self.sources.push(Source { path: path.into(), contents: contents.into() });
        id
    }

    /// The source with the given id, which must come from this map.
    pub fn get(&self, id: SourceId) -> &Source {
        &self.sources[id.0 as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gives_each_source_its_own_id() {
        let mut sources = SourceMap::new();
        let config = sources.add("zenith.toml", "[files]\n");
        let module = sources.add("web/src/a.ts", "import './b';\n");

        assert_ne!(config, module);
        assert_eq!(sources.get(config).path, "zenith.toml");
        assert_eq!(sources.get(module).contents, "import './b';\n");
    }
}
