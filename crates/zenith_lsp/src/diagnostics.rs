//! Reports as the diagnostics of the Language Server Protocol: on the files
//! as the editor names them, at positions in the encoding agreed with it.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use lsp_types::{
    ClientCapabilities, Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity,
    DiagnosticTag, Location, NumberOrString, Position, PositionEncodingKind, Range, Url,
};
use zenith_diagnostics::{
    Columns, Report, ReportElement, ReportKind, ReportNoteKind, Source, SourceMap, Span,
};
use zenith_utils::{fs::adjust_canonicalisation, location::ByteRange};

/// What the diagnostics name as where they come from.
const SOURCE: &str = "zenith";

/// The folder that the editor opened, by which it names the files in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Folder {
    /// Its path, as the editor names it, which may go through symlinks.
    pub(crate) path: PathBuf,

    /// Its canonical path, as zenith names the files in it.
    canonical: PathBuf,
}

impl Folder {
    /// The folder at `path`, as the editor names it.
    fn new(path: PathBuf) -> Self {
        Self { canonical: adjust_canonicalisation(&path), path }
    }

    /// The URI of the file at `path`, a canonical path: under the folder's
    /// path as the editor names it, if the file is in it, and `None` if the
    /// path is not absolute.
    fn uri(&self, path: &Path) -> Option<Url> {
        match path.strip_prefix(&self.canonical) {
            Ok(relative) => Url::from_file_path(self.path.join(relative)).ok(),
            Err(_) => Url::from_file_path(path).ok(),
        }
    }
}

/// The diagnostics of some reports, by the URI of their file, and the
/// reports that have none.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Diagnostics<'r> {
    /// The diagnostics of each file that has any.
    pub(crate) files: BTreeMap<Url, Vec<Diagnostic>>,

    /// The reports without an excerpt in a file that has a URI.
    pub(crate) unlocated: Vec<&'r Report>,
}

/// How the editor shows diagnostics: what the characters of positions count,
/// and the folder that it opened, if it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Editor {
    /// What the characters of positions count.
    pub(crate) columns: Columns,

    /// The folder that the editor opened, if it said.
    pub(crate) folder: Option<Folder>,
}

impl Editor {
    /// An editor of `capabilities`, which opened `folder`. Positions count
    /// UTF-8 bytes if it supports them, or else characters, and otherwise
    /// UTF-16 code units, the protocol's default.
    pub(crate) fn new(capabilities: &ClientCapabilities, folder: Option<PathBuf>) -> Self {
        let general = capabilities.general.as_ref();
        let encodings = general.and_then(|general| general.position_encodings.as_ref());
        let supports = |kind| encodings.is_some_and(|encodings| encodings.contains(&kind));
        let columns =
            match (supports(PositionEncodingKind::UTF8), supports(PositionEncodingKind::UTF32)) {
                (true, _) => Columns::Utf8,
                (false, true) => Columns::Chars,
                (false, false) => Columns::Utf16,
            };
        Self { columns, folder: folder.map(Folder::new) }
    }

    /// What the characters of positions count, as the protocol names it.
    pub(crate) fn position_encoding(&self) -> PositionEncodingKind {
        match self.columns {
            Columns::Utf8 => PositionEncodingKind::UTF8,
            Columns::Utf16 => PositionEncodingKind::UTF16,
            Columns::Chars => PositionEncodingKind::UTF32,
        }
    }

    /// The diagnostics of `reports`, and of `unnecessary`, the reports of
    /// code that has no effect, which point into `sources`. A report has none
    /// if it has no excerpt, or if its file has no URI. Code that has no
    /// effect is a hint, tagged for editors to fade it.
    pub(crate) fn diagnostics<'r>(
        &self,
        sources: &SourceMap,
        reports: &'r [Report],
        unnecessary: &'r [Report],
    ) -> Diagnostics<'r> {
        let mut diagnostics = Diagnostics::default();
        let tagged = reports.iter().map(|report| (report, false));
        for (report, unneeded) in tagged.chain(unnecessary.iter().map(|report| (report, true))) {
            let Some((uri, mut diagnostic)) = self.diagnostic(sources, report) else {
                diagnostics.unlocated.push(report);
                continue;
            };
            if unneeded {
                diagnostic.severity = Some(DiagnosticSeverity::HINT);
                diagnostic.tags = Some(vec![DiagnosticTag::UNNECESSARY]);
            }
            diagnostics.files.entry(uri).or_default().push(diagnostic);
        }
        diagnostics
    }

    /// The diagnostic of `report`, and the URI of the file it is on: at its
    /// first excerpt, with its other excerpts as related information, and its
    /// help after its title. The contract or other subject of the report is
    /// its code.
    fn diagnostic(&self, sources: &SourceMap, report: &Report) -> Option<(Url, Diagnostic)> {
        let mut blocks = report.contents.iter().filter_map(|element| match element {
            ReportElement::CodeBlock(block) => Some(block),
            ReportElement::Note(_) => None,
        });
        let Location { uri, range } = self.location(sources, blocks.next()?.span)?;
        let related: Vec<_> = blocks
            .filter_map(|block| {
                Some(DiagnosticRelatedInformation {
                    location: self.location(sources, block.span)?,
                    message: block.label.clone().unwrap_or_else(|| report.title.clone()),
                })
            })
            .collect();
        let mut message = report.title.clone();
        for element in &report.contents {
            if let ReportElement::Note(note) = element
                && note.kind == ReportNoteKind::Help
            {
                message = format!("{message}\n{}", note.message);
            }
        }
        let diagnostic = Diagnostic {
            range,
            severity: Some(match report.kind {
                ReportKind::Error => DiagnosticSeverity::ERROR,
                ReportKind::Warning => DiagnosticSeverity::WARNING,
            }),
            code: report.id.clone().map(NumberOrString::String),
            source: Some(SOURCE.to_string()),
            message,
            related_information: (!related.is_empty()).then_some(related),
            ..Diagnostic::default()
        };
        Some((uri, diagnostic))
    }

    /// Where `span` is: the URI of its file, as the editor names it, and its
    /// range. `None` if the file has no URI, as a path that is not absolute
    /// has none.
    fn location(&self, sources: &SourceMap, span: Span) -> Option<Location> {
        let path = sources.disk_path(span.source);
        let uri = match &self.folder {
            Some(folder) => folder.uri(&path)?,
            None => Url::from_file_path(path).ok()?,
        };
        let range = self.range(sources.get(span.source), span.range);
        Some(Location { uri, range })
    }

    /// The range of the bytes `range` of `source`.
    fn range(&self, source: &Source, range: ByteRange) -> Range {
        Range {
            start: self.position(source, range.start()),
            end: self.position(source, range.end()),
        }
    }

    /// The position of the byte `offset` of `source`: its 0-based line, and
    /// the characters before it in the line.
    fn position(&self, source: &Source, offset: u32) -> Position {
        let position = source.position_in(offset, self.columns);
        Position { line: position.line - 1, character: position.column - 1 }
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use lsp_types::GeneralClientCapabilities;

    use super::*;

    /// A line with characters of one, two and four bytes, which are one or
    /// two UTF-16 code units, before an import.
    const TEXT: &str = "// zenith\n/* café 😀 */ import x from './x';\n";

    /// A project root, which must be absolute for its files to have URIs.
    fn sources() -> (SourceMap, Span) {
        let mut sources = SourceMap::with_root(&env::temp_dir());
        let source = sources.add("web/a.ts", TEXT);
        let start = TEXT.find("'./x'").unwrap() as u32;
        (sources, Span::new(source, ByteRange::new(start, start + 5)))
    }

    /// An editor of UTF-16 positions that opened no folder.
    const UTF16: Editor = Editor { columns: Columns::Utf16, folder: None };

    #[test]
    fn negotiates_utf8_when_the_editor_supports_it() {
        let columns = |encodings: Option<Vec<PositionEncodingKind>>| {
            let capabilities = ClientCapabilities {
                general: Some(GeneralClientCapabilities {
                    position_encodings: encodings,
                    ..GeneralClientCapabilities::default()
                }),
                ..ClientCapabilities::default()
            };
            Editor::new(&capabilities, None).columns
        };
        let (utf8, utf16, utf32) =
            (PositionEncodingKind::UTF8, PositionEncodingKind::UTF16, PositionEncodingKind::UTF32);
        let all = vec![utf16.clone(), utf32.clone(), utf8];
        assert_eq!(columns(Some(all)), Columns::Utf8);
        assert_eq!(columns(Some(vec![utf16.clone(), utf32])), Columns::Chars);
        assert_eq!(columns(Some(vec![utf16])), Columns::Utf16);
        assert_eq!(columns(None), Columns::Utf16);
        assert_eq!(Editor::new(&ClientCapabilities::default(), None).columns, Columns::Utf16);
    }

    #[test]
    fn positions_count_the_characters_of_their_encoding() {
        let (sources, span) = sources();
        let source = sources.get(span.source);
        // `/* café 😀 */ ` is 13 characters: 17 bytes, and 14 UTF-16 code units.
        let range = |start, end| Range::new(Position::new(1, start), Position::new(1, end));
        let editor = |columns| Editor { columns, folder: None };
        for (columns, expected, encoding) in [
            (Columns::Utf8, range(31, 36), PositionEncodingKind::UTF8),
            (Columns::Utf16, range(28, 33), PositionEncodingKind::UTF16),
            (Columns::Chars, range(27, 32), PositionEncodingKind::UTF32),
        ] {
            assert_eq!(editor(columns).range(source, span.range), expected);
            assert_eq!(editor(columns).position_encoding(), encoding);
        }
        assert_eq!(UTF16.position(source, 0), Position::new(0, 0));
    }

    #[test]
    fn reports_are_diagnostics_on_their_first_excerpt() {
        let (sources, span) = sources();
        let mut report = Report::error("`web` must not import `api`").with_id("web-layers");
        report.add_span(span).add_note("not shown").add_help("move it down a layer");
        let config = Span::new(span.source, ByteRange::new(0, 9));
        report.add_labelled_span(config, "the comment");
        let unlocated = Report::warning("no excerpt");
        let mut unused = Report::warning("the suppression has no effect");
        unused.add_span(config);

        let reports = [report, unlocated];
        let diagnostics = UTF16.diagnostics(&sources, &reports, std::slice::from_ref(&unused));
        let uri = Url::from_file_path(env::temp_dir().join("web/a.ts")).unwrap();
        let range =
            |line, start, end| Range::new(Position::new(line, start), Position::new(line, end));
        let expected = Diagnostic {
            range: range(1, 28, 33),
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::String("web-layers".into())),
            source: Some("zenith".into()),
            message: "`web` must not import `api`\nmove it down a layer".into(),
            related_information: Some(vec![DiagnosticRelatedInformation {
                location: Location { uri: uri.clone(), range: range(0, 0, 9) },
                message: "the comment".into(),
            }]),
            ..Diagnostic::default()
        };
        // Code that has no effect is faded.
        let unnecessary = Diagnostic {
            range: range(0, 0, 9),
            severity: Some(DiagnosticSeverity::HINT),
            source: Some("zenith".into()),
            message: "the suppression has no effect".into(),
            tags: Some(vec![DiagnosticTag::UNNECESSARY]),
            ..Diagnostic::default()
        };
        let files = BTreeMap::from([(uri, vec![expected, unnecessary])]);
        assert_eq!(diagnostics, Diagnostics { files, unlocated: vec![&reports[1]] });
    }

    #[test]
    fn files_without_an_absolute_path_have_no_diagnostics() {
        let mut sources = SourceMap::new();
        let source = sources.add("web/a.ts", TEXT);
        let mut report = Report::error("`web` must not import `api`");
        report.add_span(Span::new(source, ByteRange::new(0, 9)));
        let reports = [report];
        let diagnostics = UTF16.diagnostics(&sources, &reports, &[]);
        assert_eq!(
            diagnostics,
            Diagnostics { files: BTreeMap::new(), unlocated: vec![&reports[0]] }
        );
    }

    #[test]
    fn files_are_named_as_the_editor_names_its_folder() {
        // The editor opened the project through a path that zenith does not name
        // it by.
        let root = adjust_canonicalisation(env::temp_dir());
        let mut sources = SourceMap::with_root(&root);
        let source = sources.add("web/a.ts", TEXT);
        let folder = Folder { path: env::temp_dir().join("link"), canonical: root };
        let editor = Editor { columns: Columns::Utf16, folder: Some(folder) };
        let span = Span::new(source, ByteRange::new(0, 9));
        let location = editor.location(&sources, span).unwrap();
        let named = env::temp_dir().join("link/web/a.ts");
        assert_eq!(location.uri, Url::from_file_path(named).unwrap());

        // A file outside the folder keeps its own path.
        let mut sources = SourceMap::new();
        let outside = env::temp_dir().parent().unwrap().join("zenith.toml");
        let source = sources.add(outside.display().to_string(), "");
        let location = editor.location(&sources, Span::new(source, ByteRange::new(0, 0))).unwrap();
        assert_eq!(location.uri, Url::from_file_path(outside).unwrap());
    }
}
