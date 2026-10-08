//! Reading source files for code excerpts.

use std::{cell::RefCell, collections::HashMap, fs, ops::Range, path::Path};

use zenith_contracts::LinkImport;

/// A cache of source files, read on demand.
pub(crate) struct Sources<'a> {
    root: &'a Path,
    files: RefCell<HashMap<String, Option<String>>>,
}

impl<'a> Sources<'a> {
    pub(crate) fn new(root: &'a Path) -> Self {
        Self { root, files: RefCell::new(HashMap::new()) }
    }

    /// The contents of a file relative to the root, if it can be read.
    pub(crate) fn get(&self, file: &str) -> Option<String> {
        self.files
            .borrow_mut()
            .entry(file.to_string())
            .or_insert_with(|| {
                fs::read(self.root.join(file))
                    .ok()
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
            })
            .clone()
    }
}

/// The span to underline for an import: the quoted specifier if it can be
/// found inside the statement, otherwise the statement's first line.
pub(crate) fn specifier_span(source: &str, import: &LinkImport) -> Range<usize> {
    let start = (import.start as usize).min(source.len());
    let end = (import.end as usize).clamp(start, source.len());
    let statement = &source[start..end];
    for quote in ['"', '\'', '`'] {
        let needle = format!("{quote}{}{quote}", import.specifier);
        if let Some(offset) = statement.find(&needle) {
            return start + offset..start + offset + needle.len();
        }
    }
    let line_end = statement.find('\n').map_or(end, |offset| start + offset);
    start..line_end.max(start + 1).min(source.len().max(start + 1))
}
