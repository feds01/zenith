//! The syntax tree visitor that finds imports and classifies them by kind.

use oxc_ast::ast::{
    Argument, CallExpression, ExportAllDeclaration, ExportFromDeclaration, Expression,
    ImportDeclaration, ImportDeclarationSpecifier, ImportExpression, ObjectPropertyKind,
    TSImportEqualsDeclaration, TSImportType, TSModuleReference,
};
use oxc_ast_visit::{Visit, walk};
use oxc_span::Span;
use oxc_str::JSStr;
use thin_vec::ThinVec;
use zenith_graph::{ImportDetail, ImportKind, ImportedName, ImportedNames};
use zenith_utils::location::Located;

use super::convert::IntoLang;

/// The calls through which a test reaches a module, as the object, the method
/// called on it and the kind of import it makes: Vitest's, Jest's, and Bun's
/// and Node's `mock.module`. Most mock the module, or make a mocked copy of
/// it, while `importActual` and `requireActual` load the module itself, past
/// its mock.
const TEST_CALLS: [(&str, &str, ImportKind); 12] = [
    ("vi", "mock", ImportKind::Mock),
    ("vi", "doMock", ImportKind::Mock),
    ("vi", "importMock", ImportKind::Mock),
    ("vi", "importActual", ImportKind::Dynamic),
    ("jest", "mock", ImportKind::Mock),
    ("jest", "doMock", ImportKind::Mock),
    ("jest", "setMock", ImportKind::Mock),
    ("jest", "unstable_mockModule", ImportKind::Mock),
    ("jest", "requireMock", ImportKind::Mock),
    ("jest", "createMockFromModule", ImportKind::Mock),
    ("jest", "requireActual", ImportKind::Require),
    ("mock", "module", ImportKind::Mock),
];

/// The kind of import that `callee` makes, if it is one of the
/// [`TEST_CALLS`]. Its object may be the property of another, as in Node's
/// `t.mock.module("s")`.
fn test_call_kind(callee: &Expression) -> Option<ImportKind> {
    let Expression::StaticMemberExpression(member) = callee else {
        return None;
    };
    let object = match &member.object {
        Expression::Identifier(object) => &object.name,
        Expression::StaticMemberExpression(object) => &object.property.name,
        _ => return None,
    };
    TEST_CALLS
        .iter()
        .find(|(name, method, _)| object == name && member.property.name == *method)
        .map(|(_, _, kind)| *kind)
}

/// Whether a mock's `options` make it virtual, as Jest's `{ virtual: true }`
/// does: it stands in for a module that doesn't exist, so it imports nothing.
fn is_virtual(options: Option<&Argument>) -> bool {
    let Some(Argument::ObjectExpression(options)) = options else {
        return false;
    };
    options.properties.iter().any(|property| {
        matches!(property, ObjectPropertyKind::ObjectProperty(property)
            if property.key.is_specific_id("virtual")
                && matches!(&property.value, Expression::BooleanLiteral(value) if value.value))
    })
}

/// Collects the imports of one file as it walks the syntax tree.
#[derive(Default)]
pub(crate) struct ImportVisitor {
    /// The imports found so far.
    imports: Vec<ImportDetail>,

    /// The kinds of the imports with non-literal specifiers found so far.
    non_literal_imports: ThinVec<Located<ImportKind>>,

    /// Whether to record the names that imports take, rather than take them
    /// all.
    names: bool,
}

impl ImportVisitor {
    /// A visitor that records the names that imports take if `names` is set.
    pub(crate) fn new(names: bool) -> Self {
        Self { names, ..Self::default() }
    }

    /// The imports and the imports with non-literal specifiers, each in
    /// source order.
    pub(crate) fn finish(mut self) -> (Vec<ImportDetail>, ThinVec<Located<ImportKind>>) {
        // The tree is walked in source order, so these are almost always
        // sorted already, which the sort detects in linear time.
        self.imports.sort_by_key(|import| import.range);
        self.non_literal_imports.sort();
        (self.imports, self.non_literal_imports)
    }

    /// What an import statement takes: its `names`, if they are recorded.
    fn names(&self, names: impl FnOnce() -> ImportedNames) -> ImportedNames {
        match self.names {
            true => names(),
            false => ImportedNames::All,
        }
    }

    /// Record an import of `specifier` made by the code at `span`, which
    /// takes the `names` of the module. A specifier with a lone surrogate,
    /// which no file can be named by, is kept with it escaped, so that it is
    /// reported as unresolved.
    fn import(&mut self, specifier: JSStr<'_>, kind: ImportKind, span: Span, names: ImportedNames) {
        let specifier = match specifier.as_str() {
            Some(specifier) => specifier.into(),
            None => specifier.display().to_string().into(),
        };
        self.imports.push(ImportDetail { range: span.into_lang(), specifier, kind, names });
    }

    /// Record a dynamic import, `require` or test call made by the code at
    /// `span`, whose specifier argument is `argument`. The specifier must
    /// be a string literal, or a template literal without substitutions, to
    /// be resolved.
    fn call(&mut self, argument: Option<&Expression>, kind: ImportKind, span: Span) {
        let specifier = match argument {
            Some(Expression::StringLiteral(literal)) => Some(literal.value),
            Some(Expression::TemplateLiteral(template)) => template.single_quasi(),
            _ => None,
        };
        match specifier {
            // How the module is used can't be told from the call.
            Some(specifier) => self.import(specifier, kind, span, ImportedNames::All),
            None => {
                self.non_literal_imports.push(Located::new(span.into_lang(), kind));
            }
        }
    }
}

// None of the import statements contain an import, so the visitor does not
// walk into them, but the expressions and import types may.
impl<'a> Visit<'a> for ImportVisitor {
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        // Type-only when every specifier is marked `type`. A default or
        // namespace import can't be marked `type` on its own.
        let type_only = it.import_kind.is_type()
            || it.specifiers.as_ref().is_some_and(|specifiers| {
                !specifiers.is_empty()
                    && specifiers.iter().all(|specifier| {
                        matches!(
                            specifier,
                            ImportDeclarationSpecifier::ImportSpecifier(specifier)
                                if specifier.import_kind.is_type()
                        )
                    })
            });
        // A namespace import takes everything, and so does a side-effect import.
        let names = self.names(|| match &it.specifiers {
            Some(specifiers)
                if !specifiers.iter().any(|specifier| {
                    matches!(specifier, ImportDeclarationSpecifier::ImportNamespaceSpecifier(_))
                }) =>
            {
                ImportedNames::of(specifiers.iter().map(|specifier| match specifier {
                    ImportDeclarationSpecifier::ImportSpecifier(specifier) => ImportedName::new(
                        specifier.imported.name().as_str(),
                        it.import_kind.is_type() || specifier.import_kind.is_type(),
                    ),
                    // A default import: a namespace import took `All`.
                    _ => ImportedName::new("default", it.import_kind.is_type()),
                }))
            }
            _ => ImportedNames::All,
        });
        self.import(it.source.value, ImportKind::of_static(type_only), it.span, names);
    }

    fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
        let type_only = it.export_kind.is_type()
            || (!it.specifiers.is_empty()
                && it.specifiers.iter().all(|specifier| specifier.export_kind.is_type()));
        // It takes what it re-exports: `a` from `export { a as b } from "./m"`.
        let names = self.names(|| {
            let names = it.specifiers.iter().map(|specifier| {
                let type_only = it.export_kind.is_type() || specifier.export_kind.is_type();
                ImportedName::new(specifier.local.name().as_str(), type_only)
            });
            ImportedNames::of(names)
        });
        self.import(it.source.value, ImportKind::of_static(type_only), it.span, names);
    }

    fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
        let kind = ImportKind::of_static(it.export_kind.is_type());
        self.import(it.source.value, kind, it.span, ImportedNames::All);
    }

    fn visit_ts_import_equals_declaration(&mut self, it: &TSImportEqualsDeclaration<'a>) {
        // `import x = require("s")`, rather than an alias like `import x = A.B`.
        if let TSModuleReference::ExternalModuleReference(reference) = &it.module_reference {
            let kind =
                if it.import_kind.is_type() { ImportKind::Type } else { ImportKind::Require };
            self.import(reference.expression.value, kind, it.span, ImportedNames::All);
        }
    }

    fn visit_ts_import_type(&mut self, it: &TSImportType<'a>) {
        self.import(it.source.value, ImportKind::Type, it.span, ImportedNames::All);
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
        } else if let Some(kind) = test_call_kind(&it.callee)
            && let [argument, rest @ ..] = it.arguments.as_slice()
            && !is_virtual(rest.get(1))
        {
            // Vitest also takes the module of a mock as `import("s")`, which
            // names it rather than loading it, so it isn't a dynamic import too.
            if let Some(Expression::ImportExpression(import)) = argument.as_expression() {
                self.call(Some(&import.source), kind, it.span);
                if let Some(type_arguments) = &it.type_arguments {
                    self.visit_ts_type_parameter_instantiation(type_arguments);
                }
                for argument in rest {
                    self.visit_argument(argument);
                }
                return;
            }
            self.call(argument.as_expression(), kind, it.span);
        }
        walk::walk_call_expression(self, it);
    }
}

#[cfg(test)]
mod tests {
    use std::{ops::Range, path::Path};

    use super::*;
    use crate::{ParseOptions, Parser};

    /// The specifier and kind of each import in `source`, parsed as `path`.
    fn kinds(path: &str, source: &str) -> Vec<(String, ImportKind)> {
        let parsed = Parser::new().parse_source(Path::new(path), source);
        assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
        parsed
            .imports
            .into_iter()
            .map(|import| (import.specifier.to_string(), import.kind))
            .collect()
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
    fn imports_name_what_they_take() {
        /// The names that the one import in `source` takes, with `:type` after
        /// those whose type only it takes, or `None` for all of them.
        fn names(source: &str) -> Option<Vec<String>> {
            let parsed = Parser::with_options(ParseOptions { names: true })
                .parse_source(Path::new("x.ts"), source);
            assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
            match &parsed.imports[0].names {
                ImportedNames::All => None,
                ImportedNames::Names(names) => Some(
                    names
                        .iter()
                        .map(|name| match name.type_only {
                            true => format!("{}:type", name.name),
                            false => name.name.to_string(),
                        })
                        .collect(),
                ),
            }
        }

        let some = |names: &[&str]| Some(names.iter().map(|name| name.to_string()).collect());
        assert_eq!(names("import a, { b, c as d } from 's';"), some(&["default", "b", "c"]));
        assert_eq!(names("import { type A, b } from 's';"), some(&["A:type", "b"]));
        assert_eq!(names("import type { A } from 's';"), some(&["A:type"]));
        assert_eq!(names("export { a as b, type C } from 's';"), some(&["a", "C:type"]));
        assert_eq!(names("export { default } from 's';"), some(&["default"]));
        // These use the whole module, or run it.
        for source in [
            "import * as ns from 's';",
            "import a, * as ns from 's';",
            "import 's';",
            "import {} from 's';",
            "export * from 's';",
            "export * as ns from 's';",
            "export {} from 's';",
            "const a = await import('s');",
            "const a = require('s');",
        ] {
            assert_eq!(names(source), None, "{source}");
        }
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
    fn mock_calls() {
        use ImportKind::*;
        check(
            "x.test.ts",
            &[
                ("vi.mock('s');", Mock),
                ("vi.mock(`s`, () => ({ a: 1 }));", Mock),
                ("vi.mock('s', { spy: true });", Mock),
                ("vi.mock(import('s'), async (original) => ({ ...(await original()) }));", Mock),
                ("vi.doMock('s');", Mock),
                ("const a = await vi.importMock('s');", Mock),
                ("jest.mock('s', () => ({}), { virtual: false });", Mock),
                ("jest.doMock('s');", Mock),
                ("jest.unstable_mockModule('s', () => ({}));", Mock),
                ("jest.setMock('s', { a: 1 });", Mock),
                ("const a = jest.requireMock('s');", Mock),
                ("const a = jest.createMockFromModule('s');", Mock),
                ("import.meta.jest.mock('s', () => ({}));", Mock),
                ("mock.module('s', () => ({}));", Mock),
                ("t.mock.module('s', { namedExports: {} });", Mock),
            ],
        );
        // A factory's own imports are found too.
        assert_eq!(
            kinds("x.test.ts", "vi.mock(import('a'), () => import('b'));"),
            [("a".to_string(), Mock), ("b".to_string(), Dynamic)]
        );
        assert_eq!(
            kinds("x.test.ts", "vi.mock('a', () => require('b'));"),
            [("a".to_string(), Mock), ("b".to_string(), Require)]
        );
        assert_eq!(
            kinds("x.test.ts", "vi.mock<typeof import('t')>(import('a'));"),
            [("a".to_string(), Mock), ("t".to_string(), Type)]
        );
        // A virtual mock stands in for a module that doesn't exist.
        assert_eq!(kinds("x.test.js", "jest.mock('s', () => ({}), { virtual: true });"), []);
    }

    #[test]
    fn actual_imports() {
        use ImportKind::*;
        check(
            "x.test.ts",
            &[
                ("const a = await vi.importActual('s');", Dynamic),
                ("const a = await vi.importActual<typeof A>(`s`);", Dynamic),
                ("const a = jest.requireActual('s');", Require),
            ],
        );
        // A mock that keeps some of the module's real exports.
        assert_eq!(
            kinds("x.test.ts", "vi.mock('a', async () => ({ ...(await vi.importActual('a')) }));"),
            [("a".to_string(), Mock), ("a".to_string(), Dynamic)]
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
            "require.resolve('s');",
            "module.require('s');",
            "a.require('s');",
            "require('s', 'extra');",
            "require();",
            "const a = 'import(\"s\")';",
            "// import a from 's';",
            "const url = import.meta.url;",
            "import a = A.B;",
            "vi.unmock('s');",
            "vi.mock();",
        ] {
            let parsed = Parser::new().parse_source(Path::new("x.ts"), source);
            assert_eq!(parsed.imports, [], "{source}");
            assert_eq!(parsed.non_literal_imports, [], "{source}");
        }
    }

    #[test]
    fn a_specifier_with_a_lone_surrogate_is_kept_escaped() {
        for source in ["import '\\uD800';", "import('\\uD800');", "require(`\\uD800`);"] {
            let parsed = Parser::new().parse_source(Path::new("x.ts"), source);
            let specifiers: Vec<_> = parsed.imports.iter().map(|i| i.specifier.as_str()).collect();
            assert_eq!(specifiers, ["\\ud800"], "{source}");
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
vi.mock(name);
const g = jest.requireActual(name);
";
        let parsed = Parser::new().parse_source(Path::new("x.ts"), source);
        assert_eq!(kinds("x.ts", source), [("./f".to_string(), Dynamic)]);
        let found: Vec<_> = parsed
            .non_literal_imports
            .iter()
            .map(|import| (import.value, &source[Range::from(import.range)]))
            .collect();
        assert_eq!(
            found,
            [
                (Dynamic, "import(name)"),
                (Dynamic, "import(`./${name}`)"),
                (Require, "require(name)"),
                (Require, "require('./' + name)"),
                (Require, "require(...names)"),
                (Mock, "vi.mock(name)"),
                (Require, "jest.requireActual(name)")
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
