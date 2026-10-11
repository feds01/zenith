//! Import extraction for JavaScript and TypeScript files.
//!
//! [`ParsedFile::parse_file`] reads a source file and
//! [`ParsedFile::parse_source`] parses source text with
//! [oxc](https://oxc.rs), then walks the syntax tree for every form that
//! imports a module, classifying each by its [`ImportKind`]. Alongside the
//! imports, the [`ParsedFile`] holds the file's directive prologue (e.g.
//! `"use client"`), its `zenith-ignore` suppression comments, the dynamic
//! imports and `require` calls whose specifier is not a literal, and any
//! syntax errors. Everything is located by its byte range in the file.
//!
//! Parsing is the hottest part of a run: every source file of a monorepo is
//! parsed, in parallel. Each thread parses into its own arena, which is reset
//! rather than freed between files, and the results own nothing from it.

mod suppressions;
mod visitor;

use std::{borrow::Cow, cell::RefCell, fmt, io, mem, path::Path};

use oxc_allocator::Allocator;
use oxc_ast_visit::Visit;
use oxc_parser::{ParseOptions, Parser};
use oxc_span::{LabeledSpan, SourceType, Span};
pub use suppressions::Suppression;
use thin_vec::ThinVec;
use zenith_diagnostics::{self as diagnostics, Report, SourceId};
use zenith_graph::{ImportDetail, ImportKind};
use zenith_utils::{fs::read_text, location::ByteRange};

use crate::visitor::ImportVisitor;

/// The extensions, without the `.`, of the files that zenith parses as
/// JavaScript or TypeScript. Declaration files (`.d.ts`) are `ts` files.
pub const SOURCE_EXTENSIONS: &[&str] = &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];

/// What zenith learns from one source file.
///
/// Every list but the imports is empty in most files, so those are
/// [`ThinVec`]s, which take one pointer each rather than three.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedFile {
    /// The imports, in source order, before their specifiers are resolved.
    pub imports: Vec<ImportDetail>,

    /// The dynamic imports and `require` calls whose specifier is not a
    /// literal, in source order. They can't be resolved, so they are not in
    /// [`imports`](Self::imports).
    pub non_literal_imports: ThinVec<NonLiteralImport>,

    /// The directive prologue, e.g. `["use client"]`: the string-literal
    /// statements at the very top of the file, each as written between its
    /// quotes.
    pub directives: ThinVec<String>,

    /// The `zenith-ignore` comments, in source order.
    pub suppressions: ThinVec<Suppression>,

    /// The syntax errors, in source order. The parser recovers from some
    /// errors and keeps what it found. When it can't, the rest of the result
    /// is empty.
    pub errors: ThinVec<SyntaxError>,
}

/// A dynamic import or `require` call whose specifier is not a string literal
/// or a template literal without substitutions, e.g. `import(path)` or
/// `` require(`./${name}`) ``.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NonLiteralImport {
    /// [`ImportKind::Dynamic`] for `import(…)`, or [`ImportKind::Require`]
    /// for `require(…)`.
    pub kind: ImportKind,

    /// Where the call is in the file.
    pub range: ByteRange,
}

/// A syntax error in a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    /// What is wrong, as the parser describes it, e.g. ``Expected `,` but
    /// found `;` ``.
    pub message: Cow<'static, str>,

    /// Where the code that the parser points at is in the file.
    pub range: ByteRange,
}

impl SyntaxError {
    /// An error with the parser's `message`, at the code its primary label
    /// points at, or its first label if none is primary.
    fn new(message: Cow<'static, str>, labels: &[LabeledSpan]) -> Self {
        let label = labels.iter().find(|label| label.primary()).or_else(|| labels.first());
        let span = label.map_or(Span::default(), LabeledSpan::span);
        Self { message, range: ByteRange::new(span.start, span.end) }
    }

    /// The report of the error, which points into `source`, the file that it
    /// is in.
    pub fn report(&self, source: SourceId) -> Report {
        let mut report = Report::error(self.message.as_ref());
        report.add_span(diagnostics::Span::new(source, self.range));
        report
    }
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SyntaxError {}

thread_local! {
    /// The arena that each thread parses into. Resetting it between files
    /// keeps its largest block of memory, so a thread soon stops allocating
    /// for its syntax trees. The memory is freed when the thread exits.
    static ALLOCATOR: RefCell<Allocator> = RefCell::new(Allocator::default());
}

impl ParsedFile {
    /// Read the source file at `path` and parse it with
    /// [`ParsedFile::parse_source`].
    ///
    /// The file must be UTF-8, or reading it fails with
    /// [`io::ErrorKind::InvalidData`]: decoding it lossily would shift the
    /// offsets of everything after the first invalid byte.
    pub fn parse_file(path: &Path) -> io::Result<Self> {
        Ok(Self::parse_source(path, &read_text(path)?))
    }

    /// Parse `source`, the text of the file at `path`, and extract its
    /// imports, directives and suppressions.
    ///
    /// ```
    /// # use std::path::Path;
    /// # use zenith_graph::ImportKind;
    /// # use zenith_parse::ParsedFile;
    /// let source = "'use client';\nimport type { A } from './a';\n";
    /// let parsed = ParsedFile::parse_source(Path::new("page.tsx"), source);
    /// assert_eq!(parsed.directives, ["use client"]);
    /// assert_eq!(parsed.imports[0].specifier, "./a");
    /// assert_eq!(parsed.imports[0].kind, ImportKind::Type);
    /// ```
    ///
    /// The extension of `path` picks the dialect: TypeScript for `.ts`, `.mts`
    /// and `.cts` (and `.d.ts` declarations), TypeScript with JSX for `.tsx`,
    /// and JavaScript with JSX otherwise. A leading byte order mark is
    /// skipped, so offsets count from the character after it.
    ///
    /// This can be called from many threads at once: each parses into its
    /// own arena. The parser and the visitor recurse into nested code, so
    /// deeply nested input, such as generated data, needs a deep stack: give
    /// worker threads more than the usual 2 MiB.
    pub fn parse_source(path: &Path, source: &str) -> Self {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        ALLOCATOR.with_borrow_mut(|allocator| {
            allocator.reset();
            Self::parse_in(allocator, source_type(path), source)
        })
    }

    /// Parse `source` into `allocator`, and collect everything zenith needs
    /// into owned values, so that nothing borrows from the arena.
    fn parse_in(allocator: &Allocator, source_type: SourceType, source: &str) -> Self {
        // 1. Parse the source into the arena.
        let options = ParseOptions {
            // CommonJS modules may `return` at the top level.
            allow_return_outside_function: true,
            allow_v8_intrinsics: true,
            // Unwrap `(…)` so that `require(("a"))` is a literal `require`.
            preserve_parens: false,
            // Nothing here hashes identifiers, so don't pay for the hashes.
            enable_ident_hashes: false,
            ..ParseOptions::default()
        };
        let parsed = Parser::new(allocator, source, source_type).with_options(options).parse();

        // 2. Collect the syntax errors. If the parser gave up, the syntax tree is
        //    empty. The comments were kept, but leave them out too, rather than report
        //    suppressions with no imports to apply to.
        let mut errors: ThinVec<SyntaxError> = parsed
            .diagnostics
            .into_iter()
            .map(|mut diagnostic| {
                SyntaxError::new(mem::take(&mut diagnostic.message), &diagnostic.labels)
            })
            .collect();
        errors.sort_by_key(|error| error.range);
        if parsed.fatal_error {
            return Self { errors, ..Self::default() };
        }

        // 3. Walk the syntax tree for the imports.
        let mut visitor = ImportVisitor::default();
        visitor.visit_program(&parsed.program);
        let (imports, non_literal_imports) = visitor.finish();

        // 4. Read the directive prologue and the suppression comments.
        let directives = parsed
            .program
            .directives
            .iter()
            .map(|directive| directive.directive.as_str().to_owned())
            .collect();
        let suppressions = Suppression::collect(source, &parsed.program.comments);

        Self { imports, non_literal_imports, directives, suppressions, errors }
    }
}

/// The dialect to parse the file at `path` in.
fn source_type(path: &Path) -> SourceType {
    match SourceType::from_path(path) {
        // JSX is common in the `.js` files of React projects.
        Ok(source_type) if source_type.is_javascript() => source_type.with_jsx(true),
        Ok(source_type) => source_type,
        Err(_) => SourceType::unambiguous().with_jsx(true),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, ops::Range, thread};

    use tempfile::TempDir;
    use zenith_diagnostics::{Reporter, SourceMap};

    use super::*;

    #[test]
    fn every_source_extension_is_a_parser_dialect() {
        for extension in SOURCE_EXTENSIONS {
            assert!(SourceType::from_extension(extension).is_ok(), "{extension}");
        }
    }

    #[test]
    fn each_extension_parses_its_own_dialect() {
        let typescript = "import type { A } from './a';\nconst b = c as A;\nexport { b };\n";
        let jsx = "import a from './a';\nexport default () => <div>{a}</div>;\n";
        let tsx = "import a from './a';\nexport const b = <T,>(x: T) => <div>{a}</div>;\n";
        let commonjs = "const a = require('./a');\nif (!a) return;\nmodule.exports = a;\n";
        let declaration = "import type { A } from './a';\nexport declare function f(): A;\n";

        for (path, source) in [
            ("x.ts", typescript),
            ("x.mts", typescript),
            ("x.cts", typescript),
            ("x.d.ts", declaration),
            ("x.tsx", tsx),
            ("x.js", jsx),
            ("x.jsx", jsx),
            ("x.mjs", jsx),
            ("x.cjs", commonjs),
            ("x.js", commonjs),
            // An unknown extension is parsed as JavaScript with JSX.
            ("x.es6", jsx),
        ] {
            let parsed = ParsedFile::parse_source(Path::new(path), source);
            assert!(parsed.errors.is_empty(), "{path}: {:?}", parsed.errors);
            assert_eq!(parsed.imports[0].specifier, "./a", "{path}");
        }
    }

    #[test]
    fn jsx_is_a_syntax_error_in_typescript_without_it() {
        let parsed = ParsedFile::parse_source(Path::new("x.ts"), "export const a = <div />;\n");
        assert!(!parsed.errors.is_empty());
    }

    #[test]
    fn records_the_directive_prologue() {
        for (source, expected) in [
            ("'use client';\nimport a from 'a';\n", &["use client"][..]),
            ("\"use server\";\n", &["use server"]),
            // Comments and a hashbang may come before the prologue.
            ("#!/usr/bin/env node\n// header\n'use strict';\n", &["use strict"]),
            // Any string is recorded, as written between its quotes.
            ("'use cache';\n'use client'\n", &["use cache", "use client"]),
            ("'use\\x20client';\n", &["use\\x20client"]),
            // The prologue ends at the first statement that isn't a string.
            ("import a from 'a';\n'use client';\n", &[]),
            ("const a = 1;\n'use client';\n", &[]),
            // A string that is part of a larger expression isn't a directive.
            ("'use client' + a;\n", &[]),
            // Directives of functions belong to the function, not the file.
            ("export async function f() {\n  'use server';\n}\n", &[]),
            ("", &[]),
        ] {
            let parsed = ParsedFile::parse_source(Path::new("x.tsx"), source);
            assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
            assert_eq!(parsed.directives, expected, "{source:?}");
        }
    }

    #[test]
    fn imports_record_their_ranges() {
        let source = "// é😀\r\nimport a from './a';\r\n\r\nconst 日本 = 1; import('./b');\r\n";
        let parsed = ParsedFile::parse_source(Path::new("x.ts"), source);
        let found: Vec<_> =
            parsed.imports.iter().map(|import| &source[Range::from(import.range)]).collect();
        assert_eq!(found, ["import a from './a';", "import('./b')"]);
    }

    #[test]
    fn a_byte_order_mark_is_skipped() {
        let source = "\u{feff}'use client';\nimport a from './a';\n";
        let parsed = ParsedFile::parse_source(Path::new("x.ts"), source);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        assert_eq!(parsed.directives, ["use client"]);
        let import = &parsed.imports[0];
        assert_eq!(import.range.start(), 14);
        assert_eq!(
            parsed,
            ParsedFile::parse_source(Path::new("x.ts"), &source['\u{feff}'.len_utf8()..])
        );
    }

    #[test]
    fn reads_files_without_their_byte_order_mark() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("page.tsx");
        fs::write(&path, "\u{feff}import a from './a';\n").unwrap();

        let parsed = ParsedFile::parse_file(&path).unwrap();
        assert_eq!(parsed.imports[0].range.start(), 0);

        let missing = ParsedFile::parse_file(&dir.path().join("missing.ts")).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn recoverable_syntax_errors_keep_the_rest() {
        let source =
            "'use client';\nimport a from './a';\nconst b;\nimport('./c'); // zenith-ignore d\n";
        let parsed = ParsedFile::parse_source(Path::new("x.ts"), source);
        let errors: Vec<_> = parsed
            .errors
            .iter()
            .map(|error| (error.to_string(), &source[Range::from(error.range)]))
            .collect();
        assert_eq!(errors, [("Missing initializer in const declaration".to_string(), "b")]);
        let specifiers: Vec<_> = parsed.imports.iter().map(|i| i.specifier.as_str()).collect();
        assert_eq!(specifiers, ["./a", "./c"]);
        assert_eq!(parsed.directives, ["use client"]);
        assert_eq!(parsed.suppressions.len(), 1);
    }

    #[test]
    fn syntax_errors_are_in_source_order_at_the_code_they_point_at() {
        // The lexer reports the escape before the parser reports `b`, and the
        // second error has a secondary label at `implements`.
        let source = "import a from './a';
const b;
const c = '\\u{zz}';
class D implements E
  extends F {}
";
        let parsed = ParsedFile::parse_source(Path::new("x.ts"), source);
        let errors: Vec<_> =
            parsed.errors.iter().map(|error| &source[Range::from(error.range)]).collect();
        assert_eq!(errors, ["b", "\\u{", "extends"]);
        assert_eq!(parsed.imports.len(), 1);
    }

    #[test]
    fn fatal_syntax_errors_are_reported_with_their_range() {
        let source = "'use client';\n// zenith-ignore a\nimport a from './a';\nconst = ;\n";
        let parsed = ParsedFile::parse_source(Path::new("x.ts"), source);
        assert_eq!(parsed.errors.len(), 1, "{:?}", parsed.errors);
        let error = &parsed.errors[0];
        assert_eq!(error.range.start() as usize, source.find("= ;").unwrap());
        // The parser gave up, so nothing else is known about the file.
        assert_eq!(parsed, ParsedFile { errors: parsed.errors.clone(), ..ParsedFile::default() });
    }

    #[test]
    fn syntax_errors_report_the_code_they_point_at() {
        let source = "import a from './a';\nconst b;\n";
        let parsed = ParsedFile::parse_source(Path::new("x.ts"), source);
        let mut sources = SourceMap::new();
        let file = sources.add("web/x.ts", source);
        let reports: Vec<_> = parsed.errors.iter().map(|error| error.report(file)).collect();
        let expected = [
            "error: Missing initializer in const declaration",
            " --> web/x.ts:2:7",
            "  |",
            "2 | const b;",
            "  |       ^",
        ];
        assert_eq!(Reporter::new(&sources, &reports).to_string(), expected.join("\n"));
    }

    #[test]
    fn the_arena_keeps_its_memory_between_files() {
        let large = "import a from './a';\n".repeat(2_000);
        let capacity = || ALLOCATOR.with_borrow(Allocator::capacity);

        // The arena grows in chunks while it warms up.
        ParsedFile::parse_source(Path::new("a.ts"), &large);
        ParsedFile::parse_source(Path::new("a.ts"), &large);
        let warm = capacity();
        assert!(warm > large.len());

        // After that, it neither frees its memory nor keeps growing.
        for _ in 0..10 {
            ParsedFile::parse_source(Path::new("b.ts"), "export * from './b';\n");
            ParsedFile::parse_source(Path::new("a.ts"), &large);
            assert!((large.len()..=warm).contains(&capacity()));
        }
    }

    #[test]
    fn results_do_not_depend_on_earlier_files_or_other_threads() {
        let first = "import a from './a';\nconst b = require('./b');\n";
        let second = "export * from './c';\n";
        let expected = (
            ParsedFile::parse_source(Path::new("a.ts"), first),
            ParsedFile::parse_source(Path::new("b.ts"), second),
        );

        thread::scope(|scope| {
            let workers: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        (0..50).all(|_| {
                            ParsedFile::parse_source(Path::new("a.ts"), first) == expected.0
                                && ParsedFile::parse_source(Path::new("b.ts"), second) == expected.1
                        })
                    })
                })
                .collect();
            for worker in workers {
                assert!(worker.join().unwrap());
            }
        });
    }
}
