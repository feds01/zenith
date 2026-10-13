//! What a file exports, read from its top-level statements, and whether it is
//! a barrel, a file that only re-exports other modules.

use compact_str::CompactString;
use oxc_ast::ast::{
    Declaration, ExportDefaultDeclarationKind, ImportDeclarationSpecifier, Program, Statement,
};
use oxc_span::Span;
use rustc_hash::FxHashMap;
use zenith_graph::{Exports, ImportDetail, ImportedName, ReExport};

/// A local name that an import statement binds: the index of the statement
/// in the file's imports, the name the source exports it as (`None` for a
/// namespace import), and whether only its type is imported.
type Binding = (u32, Option<CompactString>, bool);

/// What zenith reads from a whole program, rather than statement by statement.
pub(crate) trait ProgramHelpers {
    /// What the file of this program exports. The source of each re-export is
    /// the index of its import statement in `imports`, the file's imports
    /// in source order.
    ///
    /// The file is a barrel when it re-exports something and every statement in
    /// it is an import with bindings or an export of imported bindings, so that
    /// it runs no code of its own. A side-effect import, such as `import
    /// "./x"`, runs the module it imports, so a file with one is not a
    /// barrel.
    fn exports(&self, imports: &[ImportDetail]) -> Exports<u32>;
}

impl ProgramHelpers for Program<'_> {
    fn exports(&self, imports: &[ImportDetail]) -> Exports<u32> {
        // Each statement that imports is in `imports`, at the range of its span.
        let index_of = |span: Span| {
            imports
                .binary_search_by_key(&span.start, |import| import.range.start())
                .ok()
                .and_then(|index| u32::try_from(index).ok())
        };

        // 1. The local names that imports bind, which may be exported before the
        //    import, since imports are hoisted.
        let mut bindings: FxHashMap<&str, Binding> = FxHashMap::default();
        for statement in &self.body {
            let Statement::ImportDeclaration(import) = statement else {
                continue;
            };
            let (Some(index), Some(specifiers)) = (index_of(import.span), &import.specifiers)
            else {
                continue;
            };
            let type_only = import.import_kind.is_type();
            for specifier in specifiers {
                let binding = match specifier {
                    ImportDeclarationSpecifier::ImportSpecifier(specifier) => (
                        specifier.local.name.as_str(),
                        Some(CompactString::from(specifier.imported.name().as_str())),
                        type_only || specifier.import_kind.is_type(),
                    ),
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                        (specifier.local.name.as_str(), Some("default".into()), type_only)
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                        (specifier.local.name.as_str(), None, type_only)
                    }
                };
                let (local, imported, type_only) = binding;
                bindings.insert(local, (index, imported, type_only));
            }
        }

        // 2. What each statement exports, and whether it runs code of its own.
        let mut exports = Exports { barrel: true, ..Exports::default() };
        for statement in &self.body {
            match statement {
                Statement::ImportDeclaration(import) => {
                    // A side-effect import, or one of no names, runs the module it imports.
                    let side_effect = import.specifiers.as_ref().is_none_or(|s| s.is_empty());
                    if side_effect || index_of(import.span).is_none() {
                        exports.barrel = false;
                    }
                }
                Statement::ExportFromDeclaration(export) => {
                    let Some(source) = index_of(export.span) else {
                        exports.barrel = false;
                        continue;
                    };
                    for specifier in &export.specifiers {
                        exports.reexports.push(ReExport {
                            name: specifier.exported.name().as_str().into(),
                            source,
                            imported: Some(specifier.local.name().as_str().into()),
                            type_only: export.export_kind.is_type()
                                || specifier.export_kind.is_type(),
                        });
                    }
                }
                Statement::ExportAllDeclaration(export) => {
                    let Some(source) = index_of(export.span) else {
                        exports.barrel = false;
                        continue;
                    };
                    let type_only = export.export_kind.is_type();
                    match &export.exported {
                        // `export * as ns from "./m"` exports the whole module as `ns`.
                        Some(name) => exports.reexports.push(ReExport {
                            name: name.name().as_str().into(),
                            source,
                            imported: None,
                            type_only,
                        }),
                        None => exports.stars.push((source, type_only)),
                    }
                }
                Statement::ExportNamedDeclaration(export) => {
                    for specifier in &export.specifiers {
                        let name = CompactString::from(specifier.exported.name().as_str());
                        let type_only =
                            export.export_kind.is_type() || specifier.export_kind.is_type();
                        match bindings.get(specifier.local.name().as_str()) {
                            Some((source, imported, imported_type)) => {
                                exports.reexports.push(ReExport {
                                    name,
                                    source: *source,
                                    imported: imported.clone(),
                                    type_only: type_only || *imported_type,
                                });
                            }
                            // A binding of the file's own code, which is declared elsewhere in
                            // it, so the file is not a barrel.
                            None => {
                                exports.barrel = false;
                                exports.names.push(ImportedName { name, type_only });
                            }
                        }
                    }
                }
                Statement::ExportDefaultDeclaration(export) => {
                    let reexported = match &export.declaration {
                        ExportDefaultDeclarationKind::Identifier(identifier) => {
                            bindings.get(identifier.name.as_str())
                        }
                        _ => None,
                    };
                    match reexported {
                        Some((source, imported, type_only)) => exports.reexports.push(ReExport {
                            name: "default".into(),
                            source: *source,
                            imported: imported.clone(),
                            type_only: *type_only,
                        }),
                        None => {
                            exports.barrel = false;
                            let type_only = matches!(
                                export.declaration,
                                ExportDefaultDeclarationKind::TSInterfaceDeclaration(_)
                            );
                            exports.names.push(ImportedName::new("default", type_only));
                        }
                    }
                }
                Statement::ExportDeclaration(export) => {
                    exports.barrel = false;
                    declared_names(&export.declaration, &mut exports.names);
                }
                Statement::EmptyStatement(_) => {}
                // Any other statement is code of the file's own, including `export =`.
                _ => exports.barrel = false,
            }
        }

        // A file that re-exports nothing is no barrel, even if it has no code.
        exports.barrel &= !exports.reexports.is_empty() || !exports.stars.is_empty();
        exports
    }
}

/// Add the names that `declaration` declares to `names`: types only for
/// interfaces and type aliases.
fn declared_names(declaration: &Declaration, names: &mut thin_vec::ThinVec<ImportedName>) {
    match declaration {
        Declaration::VariableDeclaration(variables) => {
            for variable in &variables.declarations {
                for identifier in variable.id.get_binding_identifiers() {
                    names.push(ImportedName::new(identifier.name.as_str(), false));
                }
            }
        }
        Declaration::TSTypeAliasDeclaration(alias) => {
            names.push(ImportedName::new(alias.id.name.as_str(), true));
        }
        Declaration::TSInterfaceDeclaration(interface) => {
            names.push(ImportedName::new(interface.id.name.as_str(), true));
        }
        declaration => {
            if let Some(id) = declaration.id() {
                names.push(ImportedName::new(id.name.as_str(), false));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use zenith_graph::{Exports, ImportedName, ImportedNames, ReExport};

    use crate::Parser;

    /// The exports of `source`, parsed as a TypeScript file.
    fn exports(source: &str) -> Exports<u32> {
        let parsed = Parser::new().with_names(true).parse_source(Path::new("x.ts"), source);
        assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
        parsed.exports
    }

    /// A re-export of `imported` from the import at `source`, as `name`.
    fn reexport(name: &str, source: u32, imported: Option<&str>, type_only: bool) -> ReExport<u32> {
        ReExport { name: name.into(), source, imported: imported.map(Into::into), type_only }
    }

    #[test]
    fn names_are_recorded_only_when_asked_for() {
        let source = "import { a } from './a';\nexport { a };\nexport const b = 1;\n";
        let parsed = Parser::new().parse_source(Path::new("x.ts"), source);
        assert_eq!(parsed.imports[0].names, ImportedNames::All);
        assert_eq!(parsed.exports, Exports::default());
    }

    #[test]
    fn files_of_re_exports_are_barrels() {
        let barrel = exports(
            "export * from './a';\n\
             export { b, c as d } from './b';\n\
             export type { E } from './e';\n\
             export * as ns from './ns';\n\
             import { f } from './f';\n\
             import G from './g';\n\
             export { f, G as g };\n\
             export default G;\n",
        );
        assert!(barrel.barrel);
        assert_eq!(barrel.stars.as_slice(), [(0, false)]);
        assert_eq!(
            barrel.reexports.as_slice(),
            [
                reexport("b", 1, Some("b"), false),
                reexport("d", 1, Some("c"), false),
                reexport("E", 2, Some("E"), true),
                reexport("ns", 3, None, false),
                reexport("f", 4, Some("f"), false),
                reexport("g", 5, Some("default"), false),
                reexport("default", 5, Some("default"), false),
            ]
        );
        assert!(barrel.names.is_empty());

        // An export before the import that binds it re-exports it too.
        let hoisted = exports("export { a };\nimport { a } from './a';\n");
        assert!(hoisted.barrel);
        assert_eq!(hoisted.reexports.as_slice(), [reexport("a", 0, Some("a"), false)]);
    }

    #[test]
    fn files_with_code_of_their_own_are_not_barrels() {
        for source in [
            "export * from './a';\nexport const b = 1;\n",
            "export * from './a';\nconsole.log('loaded');\n",
            "export * from './a';\nimport './polyfill';\n",
            "export * from './a';\nimport {} from './polyfill';\n",
            "export * from './a';\nexport { A };\n",
            "export * from './a';\nexport default function f() {}\n",
            "export * from './a';\ninterface A {}\n",
            // Nothing re-exported, so nothing to follow.
            "import { a } from './a';\n",
            "",
        ] {
            assert!(!exports(source).barrel, "{source:?}");
        }
    }

    #[test]
    fn modules_name_what_they_declare() {
        let module = exports(
            "export const a = 1, { b, c: [d] } = e;\n\
             export function f() {}\n\
             export class G {}\n\
             export enum H {}\n\
             export interface I {}\n\
             export type J = I;\n\
             const k = 1;\n\
             export { k as l };\n\
             export default k;\n",
        );
        let names: Vec<_> =
            module.names.iter().map(|name| (name.name.as_str(), name.type_only)).collect();
        assert_eq!(
            names,
            [
                ("a", false),
                ("b", false),
                ("d", false),
                ("f", false),
                ("G", false),
                ("H", false),
                ("I", true),
                ("J", true),
                ("l", false),
                ("default", false),
            ]
        );
        assert!(!module.barrel);
        assert_eq!(
            exports("export default interface A {}\n").names.as_slice(),
            [ImportedName::new("default", true)]
        );
    }
}
