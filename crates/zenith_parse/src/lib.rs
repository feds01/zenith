//! Import extraction for JavaScript and TypeScript files.
//!
//! Each file is parsed with [oxc](https://oxc.rs), and the AST is walked for
//! every syntactic form that creates a module dependency (see
//! [ImportKind] for how they are classified). Alongside the imports, the
//! parser records the file's directive prologue (`"use client"`) and any
//! `zenith-ignore` suppression comments.

mod suppressions;
mod visitor;

use std::path::Path;

pub use oxc_allocator::Allocator;
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;
pub use suppressions::Suppression;
pub use zenith_graph::ImportKind;
use zenith_utils::text::LineIndex;

/// An import found in a source file, before resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawImport {
    /// The specifier as written, e.g. `@/domains/campaigns`.
    pub specifier: String,
    /// How the import is made.
    pub kind: ImportKind,
    /// The byte span of the import statement or expression.
    pub start: u32,
    pub end: u32,
    /// The 1-based line and column of the start of the import.
    pub line: u32,
    pub column: u32,
}

/// Everything zenith needs from one source file.
#[derive(Debug, Clone, Default)]
pub struct ParsedFile {
    /// The imports, in source order.
    pub imports: Vec<RawImport>,
    /// The directive prologue, e.g. `["use client"]`.
    pub directives: Vec<String>,
    /// `zenith-ignore` comments.
    pub suppressions: Vec<Suppression>,
    /// The number of dynamic imports and `require` calls whose specifier is
    /// not a string literal, and so cannot be resolved.
    pub non_literal_imports: u32,
    /// Syntax errors reported by the parser. Zenith still uses whatever
    /// imports were recovered.
    pub errors: Vec<String>,
}

/// The extensions zenith parses as JavaScript or TypeScript.
pub const SOURCE_EXTENSIONS: &[&str] = &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];

/// Whether a path has an extension zenith can parse.
pub fn is_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| SOURCE_EXTENSIONS.contains(&extension))
}

/// Parse a source file and extract its imports, directives and suppressions.
pub fn parse_file(path: &Path, source: &str) -> ParsedFile {
    let allocator = Allocator::default();
    parse_with_allocator(&allocator, path, source)
}

/// Like [parse_file], with a caller-provided arena so that it can be reused.
pub fn parse_with_allocator(allocator: &Allocator, path: &Path, source: &str) -> ParsedFile {
    let source_type = source_type_for(path);
    let options = ParseOptions {
        allow_return_outside_function: true,
        allow_v8_intrinsics: true,
        ..ParseOptions::default()
    };
    let parsed = Parser::new(allocator, source, source_type).with_options(options).parse();

    let lines = LineIndex::new(source);
    let mut visitor = visitor::ImportVisitor::new(source, &lines);
    visitor.visit(&parsed.program);

    let directives =
        parsed.program.directives.iter().map(|d| d.expression.value.as_str().to_string()).collect();
    let suppressions = suppressions::collect(source, &lines, &parsed.program.comments);

    let errors = parsed.diagnostics.iter().take(5).map(|error| error.to_string()).collect();

    let mut imports = visitor.imports;
    imports.sort_by_key(|import| (import.start, import.end));
    imports.dedup_by(|a, b| a.start == b.start && a.specifier == b.specifier);

    ParsedFile {
        imports,
        directives,
        suppressions,
        non_literal_imports: visitor.non_literal_imports,
        errors,
    }
}

fn source_type_for(path: &Path) -> SourceType {
    let source_type = SourceType::from_path(path).unwrap_or_default();
    // JSX is common in `.js` files of React projects.
    if source_type.is_javascript() { source_type.with_jsx(true) } else { source_type }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imports(path: &str, source: &str) -> Vec<(String, ImportKind, u32)> {
        parse_file(Path::new(path), source)
            .imports
            .into_iter()
            .map(|import| (import.specifier, import.kind, import.line))
            .collect()
    }

    fn s(specifier: &str, kind: ImportKind, line: u32) -> (String, ImportKind, u32) {
        (specifier.to_string(), kind, line)
    }

    #[test]
    fn classifies_static_imports() {
        use ImportKind::*;
        let source = r#"import a from "./a";
import type { B } from "./b";
import { type C } from "./c";
import { type D, e } from "./d";
import "./side-effect.css";
import * as ns from "./ns";
import {} from "./empty";
"#;
        assert_eq!(
            imports("x.ts", source),
            vec![
                s("./a", Value, 1),
                s("./b", Type, 2),
                s("./c", Type, 3),
                s("./d", Value, 4),
                s("./side-effect.css", Value, 5),
                s("./ns", Value, 6),
                s("./empty", Value, 7),
            ]
        );
    }

    #[test]
    fn classifies_re_exports() {
        use ImportKind::*;
        let source = r#"export * from "./a";
export * as ns from "./b";
export type * from "./c";
export { x } from "./d";
export type { Y } from "./e";
export { type Z } from "./f";
export const local = 1;
"#;
        assert_eq!(
            imports("x.ts", source),
            vec![
                s("./a", Value, 1),
                s("./b", Value, 2),
                s("./c", Type, 3),
                s("./d", Value, 4),
                s("./e", Type, 5),
                s("./f", Type, 6),
            ]
        );
    }

    #[test]
    fn classifies_dynamic_imports_and_requires() {
        use ImportKind::*;
        let source = r#"const sharp = await import("sharp");
const lazy = () => import(`./lazy`);
const nope = import(name);
const tpl = import(`./${name}`);
const plugin = require("tailwindcss/plugin");
import fs = require("node:fs");
type T = typeof import("./types");
let u: import("./u").U;
"#;
        let parsed = parse_file(Path::new("x.ts"), source);
        let found: Vec<_> =
            parsed.imports.iter().map(|i| (i.specifier.clone(), i.kind, i.line)).collect();
        assert_eq!(
            found,
            vec![
                s("sharp", Dynamic, 1),
                s("./lazy", Dynamic, 2),
                s("tailwindcss/plugin", Require, 5),
                s("node:fs", Require, 6),
                s("./types", Type, 7),
                s("./u", Type, 8),
            ]
        );
        assert_eq!(parsed.non_literal_imports, 2);
    }

    #[test]
    fn records_directives() {
        let parsed = parse_file(Path::new("x.tsx"), "\"use client\";\nimport a from 'a';\n");
        assert_eq!(parsed.directives, vec!["use client"]);
        assert!(
            parse_file(Path::new("y.ts"), "import 'a';\n'use client';\n").directives.is_empty()
        );
    }

    #[test]
    fn parses_jsx_in_js_files() {
        let parsed =
            parse_file(Path::new("x.js"), "import a from './a';\nexport default () => <div />;\n");
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        assert_eq!(parsed.imports.len(), 1);
    }

    #[test]
    fn records_columns_in_characters() {
        let parsed = parse_file(Path::new("x.ts"), "const é = 1; import('./a');\n");
        assert_eq!((parsed.imports[0].line, parsed.imports[0].column), (1, 14));
    }
}
