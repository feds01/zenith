//! The syntax tree visitor that finds imports and classifies them by kind.

use oxc_ast::ast::{
    CallExpression, ExportAllDeclaration, ExportFromDeclaration, Expression, ImportDeclaration,
    ImportDeclarationSpecifier, ImportExpression, TSImportEqualsDeclaration, TSImportType,
    TSModuleReference,
};
use oxc_ast_visit::{Visit, walk};
use oxc_span::Span;
use thin_vec::ThinVec;
use zenith_graph::{ImportDetail, ImportKind};
use zenith_utils::location::ByteRange;

use crate::NonLiteralImport;

/// Collects the imports of one file as it walks the syntax tree.
#[derive(Default)]
pub(crate) struct ImportVisitor {
    /// The imports found so far.
    imports: Vec<ImportDetail>,

    /// The imports with non-literal specifiers found so far.
    non_literal_imports: ThinVec<NonLiteralImport>,
}

impl ImportVisitor {
    /// The imports and the imports with non-literal specifiers, each in
    /// source order.
    pub(crate) fn finish(mut self) -> (Vec<ImportDetail>, ThinVec<NonLiteralImport>) {
        // The tree is walked in source order, so these are almost always
        // sorted already, which the sort detects in linear time.
        self.imports.sort_by_key(|import| import.range);
        self.non_literal_imports.sort_by_key(|import| import.range);
        (self.imports, self.non_literal_imports)
    }

    /// Record an import of `specifier` made by the code at `span`.
    fn import(&mut self, specifier: &str, kind: ImportKind, span: Span) {
        self.imports.push(ImportDetail {
            range: ByteRange::new(span.start, span.end),
            specifier: specifier.to_owned(),
            kind,
        });
    }

    /// Record a dynamic import or `require` call made by the code at `span`,
    /// whose specifier argument is `argument`.
    fn call(&mut self, argument: Option<&Expression>, kind: ImportKind, span: Span) {
        match argument.and_then(literal_specifier) {
            Some(specifier) => self.import(specifier, kind, span),
            None => self
                .non_literal_imports
                .push(NonLiteralImport { kind, range: ByteRange::new(span.start, span.end) }),
        }
    }
}

/// The value of a string literal, or of a template literal without
/// substitutions.
fn literal_specifier<'e>(expression: &'e Expression) -> Option<&'e str> {
    match expression {
        Expression::StringLiteral(literal) => Some(literal.value.as_str()),
        Expression::TemplateLiteral(template) => {
            template.single_quasi().map(|quasi| quasi.as_str())
        }
        _ => None,
    }
}

/// The kind of a static import or re-export: `type` if it imports only
/// types, and `value` otherwise.
fn static_kind(type_only: bool) -> ImportKind {
    if type_only { ImportKind::Type } else { ImportKind::Value }
}

/// Whether `specifiers` is not empty and every one of them is type-only.
fn every_type<T>(specifiers: &[T], is_type: impl Fn(&T) -> bool) -> bool {
    !specifiers.is_empty() && specifiers.iter().all(is_type)
}

// None of the import statements contain an import, so the visitor does not
// walk into them, but the expressions and import types may.
impl<'a> Visit<'a> for ImportVisitor {
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        // A default or namespace import can't be marked `type` on its own.
        let type_only = it.import_kind.is_type()
            || it.specifiers.as_ref().is_some_and(|specifiers| {
                every_type(specifiers, |specifier| {
                    matches!(
                        specifier,
                        ImportDeclarationSpecifier::ImportSpecifier(specifier)
                            if specifier.import_kind.is_type()
                    )
                })
            });
        self.import(it.source.value.as_str(), static_kind(type_only), it.span);
    }

    fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
        let type_only = it.export_kind.is_type()
            || every_type(&it.specifiers, |specifier| specifier.export_kind.is_type());
        self.import(it.source.value.as_str(), static_kind(type_only), it.span);
    }

    fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
        let kind = static_kind(it.export_kind.is_type());
        self.import(it.source.value.as_str(), kind, it.span);
    }

    fn visit_ts_import_equals_declaration(&mut self, it: &TSImportEqualsDeclaration<'a>) {
        // `import x = require("s")`, rather than an alias like `import x = A.B`.
        if let TSModuleReference::ExternalModuleReference(reference) = &it.module_reference {
            let kind =
                if it.import_kind.is_type() { ImportKind::Type } else { ImportKind::Require };
            self.import(reference.expression.value.as_str(), kind, it.span);
        }
    }

    fn visit_ts_import_type(&mut self, it: &TSImportType<'a>) {
        self.import(it.source.value.as_str(), ImportKind::Type, it.span);
        // Its type arguments may hold more import types.
        walk::walk_ts_import_type(self, it);
    }

    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        self.call(Some(&it.source), ImportKind::Dynamic, it.span);
        walk::walk_import_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if let Expression::Identifier(callee) = &it.callee
            && callee.name == "require"
            && let [argument] = it.arguments.as_slice()
        {
            self.call(argument.as_expression(), ImportKind::Require, it.span);
        }
        walk::walk_call_expression(self, it);
    }
}

#[cfg(test)]
mod tests {
    use std::{ops::Range, path::Path};

    use super::*;
    use crate::ParsedFile;

    /// The specifier and kind of each import in `source`, parsed as `path`.
    fn kinds(path: &str, source: &str) -> Vec<(String, ImportKind)> {
        let parsed = ParsedFile::parse_source(Path::new(path), source);
        assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
        parsed.imports.into_iter().map(|import| (import.specifier, import.kind)).collect()
    }

    /// Check that each source has one import, of `s` with the given kind.
    fn check(path: &str, cases: &[(&str, ImportKind)]) {
        for (source, kind) in cases {
            assert_eq!(kinds(path, source), [("s".to_string(), *kind)], "{source}");
        }
    }

    #[test]
    fn static_imports() {
        use ImportKind::*;
        check(
            "x.ts",
            &[
                ("import a from 's';", Value),
                ("import * as ns from 's';", Value),
                ("import { a, b as c } from 's';", Value),
                ("import a, { b } from 's';", Value),
                ("import 's';", Value),
                ("import {} from 's';", Value),
                ("import json from 's' with { type: 'json' };", Value),
                ("import type a from 's';", Type),
                ("import type * as ns from 's';", Type),
                ("import type { A, B } from 's';", Type),
                // Type-only when every specifier is marked `type`.
                ("import { type A } from 's';", Type),
                ("import { type A, type B as C } from 's';", Type),
                ("import { type A, b } from 's';", Value),
                ("import a, { type B } from 's';", Value),
            ],
        );
    }

    #[test]
    fn re_exports() {
        use ImportKind::*;
        check(
            "x.ts",
            &[
                ("export * from 's';", Value),
                ("export * as ns from 's';", Value),
                ("export { a } from 's';", Value),
                ("export { default } from 's';", Value),
                ("export { default as a, b } from 's';", Value),
                ("export {} from 's';", Value),
                ("export type * from 's';", Type),
                ("export type * as ns from 's';", Type),
                ("export type { A } from 's';", Type),
                ("export { type A } from 's';", Type),
                ("export { type A, type B } from 's';", Type),
                ("export { type A, b } from 's';", Value),
            ],
        );
        // Exports of local bindings import nothing.
        assert_eq!(kinds("x.ts", "const a = 1;\nexport { a };\nexport type { A };\n"), []);
    }

    #[test]
    fn dynamic_imports() {
        use ImportKind::*;
        check(
            "x.ts",
            &[
                ("const a = await import('s');", Dynamic),
                ("const a = import(`s`);", Dynamic),
                ("const a = import(('s'));", Dynamic),
                ("const a = import('s', { with: { type: 'json' } });", Dynamic),
                ("const a = () => import('s').then((m) => m.default);", Dynamic),
            ],
        );
    }

    #[test]
    fn require_calls() {
        use ImportKind::*;
        check(
            "x.cjs",
            &[
                ("const a = require('s');", Require),
                ("const a = require(`s`);", Require),
                ("const { a } = require('s').b;", Require),
                ("require('s');", Require),
            ],
        );
        check(
            "x.ts",
            &[
                ("import a = require('s');", Require),
                ("export import a = require('s');", Require),
                ("export = require('s');", Require),
                ("import type a = require('s');", Type),
            ],
        );
    }

    #[test]
    fn import_types() {
        use ImportKind::*;
        check(
            "x.ts",
            &[
                ("type A = typeof import('s');", Type),
                ("type A = import('s').A;", Type),
                ("let a: import('s').A<string>;", Type),
                ("function f(a: typeof import('s')): void {}", Type),
            ],
        );
        // An import type in the type arguments of another.
        assert_eq!(
            kinds("x.ts", "type A = import('a').A<import('b').B>;"),
            [("a".to_string(), Type), ("b".to_string(), Type)]
        );
    }

    #[test]
    fn non_imports() {
        for source in [
            "vi.mock('s');",
            "jest.mock('s', () => ({}));",
            "require.resolve('s');",
            "module.require('s');",
            "a.require('s');",
            "require('s', 'extra');",
            "require();",
            "const a = 'import(\"s\")';",
            "// import a from 's';",
            "const url = import.meta.url;",
            "import a = A.B;",
        ] {
            let parsed = ParsedFile::parse_source(Path::new("x.ts"), source);
            assert_eq!(parsed.imports, [], "{source}");
            assert_eq!(parsed.non_literal_imports, [], "{source}");
        }
    }

    #[test]
    fn non_literal_specifiers_are_listed_separately() {
        use ImportKind::*;
        let source = "const a = import(name);
const b = import(`./${name}`);
const c = require(name);
const d = require('./' + name);
const e = require(...names);
const f = import('./f');
";
        let parsed = ParsedFile::parse_source(Path::new("x.ts"), source);
        assert_eq!(kinds("x.ts", source), [("./f".to_string(), Dynamic)]);
        let found: Vec<_> = parsed
            .non_literal_imports
            .iter()
            .map(|import| (import.kind, &source[Range::from(import.range)]))
            .collect();
        assert_eq!(
            found,
            [
                (Dynamic, "import(name)"),
                (Dynamic, "import(`./${name}`)"),
                (Require, "require(name)"),
                (Require, "require('./' + name)"),
                (Require, "require(...names)")
            ]
        );
    }

    #[test]
    fn imports_nested_anywhere_are_found_in_source_order() {
        use ImportKind::*;
        let source = "import a from './a';
export const b = async () => {
  const { c } = await import('./c');
  return class extends require('./d').D {
    e = import('./e');
  };
};
let f: typeof import('./f');
";
        assert_eq!(
            kinds("x.ts", source),
            [
                ("./a".to_string(), Value),
                ("./c".to_string(), Dynamic),
                ("./d".to_string(), Require),
                ("./e".to_string(), Dynamic),
                ("./f".to_string(), Type),
            ]
        );
    }

    #[test]
    fn jsx_files_find_imports_in_expressions() {
        use ImportKind::*;
        let source = "import a from './a';
const B = lazy(() => import('./b'));
export default () => <B c={require('./c')} />;
";
        for path in ["x.jsx", "x.js", "x.tsx"] {
            assert_eq!(
                kinds(path, source),
                [
                    ("./a".to_string(), Value),
                    ("./b".to_string(), Dynamic),
                    ("./c".to_string(), Require)
                ],
                "{path}"
            );
        }
    }

    #[test]
    fn duplicate_imports_are_kept() {
        let source = "import a from 's';\nimport { b } from 's';\n";
        assert_eq!(kinds("x.ts", source).len(), 2);
    }
}
