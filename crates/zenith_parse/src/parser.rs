//! The parser, which parses JavaScript and TypeScript sources into
//! [`ParsedFile`]s.

use std::{io, mem, path::Path};

use oxc_allocator::Allocator;
use oxc_ast_visit::Visit;
use oxc_parser::ParseOptions;
use oxc_span::SourceType;
use thin_vec::ThinVec;
use zenith_utils::fs::read_text;

use crate::{ParsedFile, Suppression, SyntaxError, visitor::ImportVisitor};

/// Parses JavaScript and TypeScript files with [oxc](https://oxc.rs) into
/// [`ParsedFile`]s.
///
/// A parser owns the arena that syntax trees are allocated in. Between files
/// it resets the arena rather than freeing it, so a parser that is reused
/// soon stops allocating, and the results own nothing from it. Parsing is the
/// hottest part of a run, so keep one parser per thread, e.g. with rayon's
/// `map_init(Parser::new, |parser, path| parser.parse_file(path))`.
///
/// ```
/// # use std::path::Path;
/// # use zenith_graph::ImportKind;
/// # use zenith_parse::Parser;
/// let mut parser = Parser::new();
/// let page = parser
///     .parse_source(Path::new("page.tsx"), "'use client';\nimport type { A } from './a';\n");
/// assert_eq!(page.directives, ["use client"]);
/// assert_eq!(
///     (page.imports[0].specifier.as_str(), page.imports[0].kind),
///     ("./a", ImportKind::Type)
/// );
///
/// // The same parser, and its arena, for the next file.
/// let a = parser.parse_source(Path::new("a.ts"), "export const a = require('./b');\n");
/// assert_eq!(a.imports[0].kind, ImportKind::Require);
/// ```
///
/// The parser and its import visitor recurse into nested code, so deeply
/// nested input, such as generated data, needs a deep stack: give worker
/// threads more than the usual 2 MiB.
#[derive(Default)]
pub struct Parser {
    /// The arena that each file's syntax tree is allocated in.
    allocator: Allocator,
}

impl Parser {
    /// A parser with an empty arena.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the source file at `path` and parse it with
    /// [`Parser::parse_source`].
    ///
    /// The file must be UTF-8, or reading it fails with
    /// [`io::ErrorKind::InvalidData`]: decoding it lossily would shift the
    /// offsets of everything after the first invalid byte.
    pub fn parse_file(&mut self, path: &Path) -> io::Result<ParsedFile> {
        Ok(self.parse_source(path, &read_text(path)?))
    }

    /// Parse `source`, the text of the file at `path`, and extract its
    /// imports, directives and suppressions.
    ///
    /// The extension of `path` picks the dialect: TypeScript for `.ts`, `.mts`
    /// and `.cts` (and `.d.ts` declarations), TypeScript with JSX for `.tsx`,
    /// and JavaScript with JSX otherwise. A `.cjs` file may use ES module
    /// syntax too. A leading byte order mark is
    /// skipped, so offsets count from the character after it.
    pub fn parse_source(&mut self, path: &Path, source: &str) -> ParsedFile {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        self.allocator.reset();
        self.parse_in_arena(source_type(path), source)
    }

    /// Parse `source` into the arena, and collect everything zenith needs
    /// into owned values, so that nothing borrows from the arena.
    fn parse_in_arena(&self, source_type: SourceType, source: &str) -> ParsedFile {
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
        let parsed = oxc_parser::Parser::new(&self.allocator, source, source_type)
            .with_options(options)
            .parse();

        // 2. Collect the syntax errors. If the parser gave up, the syntax tree is
        //    empty. The comments were kept, but leave them out too, rather than report
        //    suppressions with no imports to apply to.
        let mut errors: ThinVec<_> = parsed
            .diagnostics
            .into_iter()
            .map(|mut diagnostic| {
                SyntaxError::located(mem::take(&mut diagnostic.message), &diagnostic.labels)
            })
            .collect();
        errors.sort_by_key(|error| error.range);
        if parsed.fatal_error {
            return ParsedFile { errors, ..ParsedFile::default() };
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

        ParsedFile { imports, non_literal_imports, directives, suppressions, errors }
    }
}

/// The dialect to parse the file at `path` in.
fn source_type(path: &Path) -> SourceType {
    match SourceType::from_path(path) {
        // Bundlers, and config loaders such as jiti, accept ES module syntax in
        // `.cjs` files, so take it there too, as they do.
        Ok(source_type) if source_type.is_javascript() && source_type.is_commonjs() => {
            source_type.with_unambiguous(true).with_jsx(true)
        }
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
    use zenith_config::SOURCE_EXTENSIONS;
    use zenith_diagnostics::{self as diagnostics, Reporter, SourceMap};

    use super::*;

    /// Parse `source` as the file at `path`, with a parser of its own.
    fn parse(path: &str, source: &str) -> ParsedFile {
        Parser::new().parse_source(Path::new(path), source)
    }

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
            let parsed = parse(path, source);
            assert!(parsed.errors.is_empty(), "{path}: {:?}", parsed.errors);
            assert_eq!(parsed.imports[0].specifier, "./a", "{path}");
        }
    }

    #[test]
    fn commonjs_files_may_use_es_module_syntax() {
        // A `tailwind.config.cjs`, say, which the config loader accepts.
        let parsed = parse("x.cjs", "import a from './a';\nexport default { a };\n");
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        assert_eq!(parsed.imports[0].specifier, "./a");
    }

    #[test]
    fn jsx_is_a_syntax_error_in_typescript_without_it() {
        let parsed = parse("x.ts", "export const a = <div />;\n");
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
            let parsed = parse("x.tsx", source);
            assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
            assert_eq!(parsed.directives, expected, "{source:?}");
        }
    }

    #[test]
    fn imports_record_their_ranges() {
        let source = "// é😀\r\nimport a from './a';\r\n\r\nconst 日本 = 1; import('./b');\r\n";
        let parsed = parse("x.ts", source);
        let found: Vec<_> =
            parsed.imports.iter().map(|import| &source[Range::from(import.range)]).collect();
        assert_eq!(found, ["import a from './a';", "import('./b')"]);
    }

    #[test]
    fn a_byte_order_mark_is_skipped() {
        let source = "\u{feff}'use client';\nimport a from './a';\n";
        let parsed = parse("x.ts", source);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        assert_eq!(parsed.directives, ["use client"]);
        let import = &parsed.imports[0];
        assert_eq!(import.range.start(), 14);
        assert_eq!(parsed, parse("x.ts", &source['\u{feff}'.len_utf8()..]));
    }

    #[test]
    fn reads_files_without_their_byte_order_mark() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("page.tsx");
        fs::write(&path, "\u{feff}import a from './a';\n").unwrap();

        let parsed = Parser::new().parse_file(&path).unwrap();
        assert_eq!(parsed.imports[0].range.start(), 0);

        let missing = Parser::new().parse_file(&dir.path().join("missing.ts")).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn recoverable_syntax_errors_keep_the_rest() {
        let source =
            "'use client';\nimport a from './a';\nconst b;\nimport('./c'); // zenith-ignore d\n";
        let parsed = parse("x.ts", source);
        let errors: Vec<_> = parsed
            .errors
            .iter()
            .map(|error| (error.value.to_string(), &source[Range::from(error.range)]))
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
        let parsed = parse("x.ts", source);
        let errors: Vec<_> =
            parsed.errors.iter().map(|error| &source[Range::from(error.range)]).collect();
        assert_eq!(errors, ["b", "\\u{", "extends"]);
        assert_eq!(parsed.imports.len(), 1);
    }

    #[test]
    fn fatal_syntax_errors_are_reported_with_their_range() {
        let source = "'use client';\n// zenith-ignore a\nimport a from './a';\nconst = ;\n";
        let parsed = parse("x.ts", source);
        assert_eq!(parsed.errors.len(), 1, "{:?}", parsed.errors);
        let error = &parsed.errors[0];
        assert_eq!(error.range.start() as usize, source.find("= ;").unwrap());
        // The parser gave up, so nothing else is known about the file.
        assert_eq!(parsed, ParsedFile { errors: parsed.errors.clone(), ..ParsedFile::default() });
    }

    #[test]
    fn syntax_errors_report_the_code_they_point_at() {
        let source = "import a from './a';\nconst b;\n";
        let parsed = parse("x.ts", source);
        let mut sources = SourceMap::new();
        let file = sources.add("web/x.ts", source);
        let reports: Vec<_> = parsed
            .errors
            .iter()
            .map(|error| error.value.report(diagnostics::Span::new(file, error.range)))
            .collect();
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
        let mut parser = Parser::new();
        let (a, b) = (Path::new("a.ts"), Path::new("b.ts"));

        // The arena grows in chunks while it warms up.
        parser.parse_source(a, &large);
        parser.parse_source(a, &large);
        let warm = parser.allocator.capacity();
        assert!(warm > large.len());

        // After that, it neither frees its memory nor keeps growing.
        for _ in 0..10 {
            parser.parse_source(b, "export * from './b';\n");
            parser.parse_source(a, &large);
            assert!((large.len()..=warm).contains(&parser.allocator.capacity()));
        }
    }

    #[test]
    fn results_do_not_depend_on_earlier_files_or_other_threads() {
        let first = "import a from './a';\nconst b = require('./b');\n";
        let second = "export * from './c';\n";
        let expected = (parse("a.ts", first), parse("b.ts", second));

        // Each thread reuses a parser of its own, as a rayon worker would.
        thread::scope(|scope| {
            let workers: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        let mut parser = Parser::new();
                        (0..50).all(|_| {
                            parser.parse_source(Path::new("a.ts"), first) == expected.0
                                && parser.parse_source(Path::new("b.ts"), second) == expected.1
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
