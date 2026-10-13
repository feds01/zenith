//! The import graph through re-exports: an import of names from a module
//! that only re-exports other modules, a barrel in JavaScript, becomes
//! imports of the modules that define those names.
//!
//! Bundlers that know a barrel has no side effects leave out the modules it
//! re-exports that nothing uses, so `import { a } from "./index"` only loads
//! the module that defines `a`. Through the barrel, the import graph has a
//! chain to every module the barrel re-exports, so contracts about what code
//! reaches what, such as "client code must not reach server code", report
//! violations that the bundle doesn't have. The graph through re-exports has
//! the same modules, and imports to where each name is defined instead.

use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{Exports, Graph, ImportDetail, ImportKind, ImportedNames, ModuleId};

/// What each module exports, by module, with the source of each re-export
/// as the node it resolved to. A module without an entry is taken to be code
/// whose exports are unknown, such as an external package.
pub type ModuleExports = FxHashMap<ModuleId, Exports<ModuleId>>;

impl Graph {
    /// This graph, with each import of names from a barrel replaced by
    /// imports of the modules that define them, following re-exports and
    /// `export *` through barrels of barrels. A name is defined by the first
    /// module on its way that isn't a barrel.
    ///
    /// An import of a barrel that takes the whole module, such as a namespace
    /// import, stays, and so does an import of a name that can't be followed:
    /// one the barrel doesn't export, or that may come from an external
    /// package through `export *`. An import of only a name's type becomes a
    /// type import.
    pub fn through_reexports(&self, exports: &ModuleExports) -> Graph {
        let names = Names { exports };
        let mut imports = Vec::with_capacity(self.detail_count());
        for (importer, imported) in self.edges() {
            for detail in self.details(importer, imported) {
                let barrel = exports.get(&imported).is_some_and(|exports| exports.barrel);
                let ImportedNames::Names(taken) = &detail.names else {
                    imports.push((importer, imported, detail.clone()));
                    continue;
                };
                if !barrel {
                    imports.push((importer, imported, detail.clone()));
                    continue;
                }
                for name in taken {
                    let mut seen = FxHashSet::default();
                    match names.definitions(imported, &name.name, name.type_only, &mut seen) {
                        Some(definitions) => {
                            for (definition, type_only) in definitions {
                                // A barrel may re-export the importer's own names.
                                if definition != importer {
                                    let detail = with_kind(detail, type_only);
                                    imports.push((importer, definition, detail));
                                }
                            }
                        }
                        None => imports.push((importer, imported, detail.clone())),
                    }
                }
            }
        }
        self.with_imports(imports)
    }
}

/// `detail`, as a type import if `type_only`.
fn with_kind(detail: &ImportDetail, type_only: bool) -> ImportDetail {
    let mut detail = detail.clone();
    if type_only {
        detail.kind = ImportKind::Type;
    }
    detail
}

/// Follows names through the exports of modules.
struct Names<'e> {
    /// What each module exports.
    exports: &'e ModuleExports,
}

impl Names<'_> {
    /// The modules that define `name` as `module` exports it, each with
    /// whether only its type gets there, or `None` if they can't be told.
    /// `type_only` is whether only its type is wanted. `seen` holds the
    /// `(barrel, name)` pairs followed so far, which a cycle of re-exports
    /// comes back to.
    fn definitions(
        &self,
        module: ModuleId,
        name: &str,
        type_only: bool,
        seen: &mut FxHashSet<(ModuleId, CompactString)>,
    ) -> Option<Vec<(ModuleId, bool)>> {
        // 1. A module of code defines what it exports.
        let Some(exports) = self.exports.get(&module).filter(|exports| exports.barrel) else {
            return Some(vec![(module, type_only)]);
        };
        if !seen.insert((module, name.into())) {
            return None;
        }

        // 2. A name the barrel re-exports by name comes from its source: the whole
        //    source, for `export * as name`.
        if let Some(reexport) = exports.reexports.iter().find(|reexport| reexport.name == name) {
            let type_only = type_only || reexport.type_only;
            return match &reexport.imported {
                Some(imported) => self.definitions(reexport.source, imported, type_only, seen),
                None => Some(vec![(reexport.source, type_only)]),
            };
        }

        // 3. Any other name, but `default`, may come through `export *` from the
        //    modules that export it. If one of them may, but can't be told to, the name
        //    can't be followed.
        if name == "default" {
            return None;
        }
        let mut definitions = Vec::new();
        for (star, star_type_only) in &exports.stars {
            match self.exports_name(*star, name, &mut FxHashSet::default()) {
                Some(true) => {
                    let type_only = type_only || *star_type_only;
                    definitions.extend(self.definitions(*star, name, type_only, seen)?);
                }
                Some(false) => {}
                None => return None,
            }
        }
        (!definitions.is_empty()).then_some(definitions)
    }

    /// Whether `module` exports `name`, or `None` if that can't be told, as
    /// for an external package. `seen` holds the modules asked about so far,
    /// which a cycle of `export *` comes back to.
    fn exports_name(
        &self,
        module: ModuleId,
        name: &str,
        seen: &mut FxHashSet<ModuleId>,
    ) -> Option<bool> {
        // 1. A module whose exports are unknown may export anything, and one asked
        //    about already doesn't add to the answer.
        let exports = self.exports.get(&module)?;
        if !seen.insert(module) {
            return Some(false);
        }

        // 2. It exports the names it defines and those it re-exports by name.
        if exports.names.iter().any(|exported| exported.name == name)
            || exports.reexports.iter().any(|reexport| reexport.name == name)
        {
            return Some(true);
        }

        // 3. Any other name, but `default`, it exports if a module it exports all of
        //    with `export *` does. If none does, but one may, that can't be told.
        if name == "default" {
            return Some(false);
        }
        let mut exported = Some(false);
        for (star, _) in &exports.stars {
            match self.exports_name(*star, name, seen) {
                Some(true) => return Some(true),
                Some(false) => {}
                None => exported = None,
            }
        }
        exported
    }
}

#[cfg(test)]
mod tests {
    use thin_vec::{ThinVec, thin_vec};
    use zenith_utils::location::ByteRange;

    use super::*;
    use crate::{GraphBuilder, ImportedName, NodeHandle, ReExport};

    /// An import statement: what it imports, with `:type` after names whose
    /// type only it takes, or `*` for the whole module.
    struct Import<'a> {
        /// The importing file.
        importer: &'a str,

        /// The imported file, or external package.
        imported: &'a str,

        /// The names it takes.
        names: &'a [&'a str],
    }

    /// The graph of `imports`, and the exports of its modules, as `exports`
    /// describes them by name.
    fn project(imports: &[Import], exports: &[(&str, Exports<&str>)]) -> (Graph, ModuleExports) {
        let mut builder = GraphBuilder::new();
        for (line, import) in (1..).zip(imports) {
            let importer = builder.add_file(import.importer).unwrap();
            let imported: NodeHandle = match import.imported.starts_with("npm:") {
                true => builder.add_external(import.imported).unwrap(),
                false => builder.add_file(import.imported).unwrap().into(),
            };
            let names = match import.names {
                ["*"] => ImportedNames::All,
                names => ImportedNames::of(
                    names
                        .iter()
                        .map(|name| match name.strip_suffix(":type") {
                            Some(name) => ImportedName::new(name, true),
                            None => ImportedName::new(*name, false),
                        })
                        .collect(),
                ),
            };
            let detail = ImportDetail {
                range: ByteRange::new(line * 100, line * 100 + 10),
                specifier: import.imported.into(),
                kind: ImportKind::Value,
                names,
            };
            builder.add_import(importer, imported, detail);
        }
        let graph = builder.finish();
        let exports = exports
            .iter()
            .map(|(module, exports)| {
                let id = |name: &str| graph.id(name).unwrap_or_else(|| panic!("no `{name}`"));
                (id(module), exports.clone().map(|source| Some(id(source))))
            })
            .collect();
        (graph, exports)
    }

    /// The imports of `graph`, as `importer -> imported` names, with `(type)`
    /// after those whose every detail is a type import.
    fn imports(graph: &Graph) -> Vec<String> {
        graph
            .edges()
            .map(|(importer, imported)| {
                let types = graph
                    .details(importer, imported)
                    .iter()
                    .all(|detail| detail.kind == ImportKind::Type);
                let suffix = if types { " (type)" } else { "" };
                format!("{} -> {}{suffix}", graph.name(importer), graph.name(imported))
            })
            .collect()
    }

    /// A barrel with these re-exports, as `(name, source, imported)`, and
    /// `export *` of the `stars`.
    fn barrel(
        reexports: &[(&'static str, &'static str, Option<&str>)],
        stars: &[&'static str],
    ) -> Exports<&'static str> {
        Exports {
            barrel: true,
            names: ThinVec::new(),
            reexports: reexports
                .iter()
                .map(|(name, source, imported)| ReExport {
                    name: (*name).into(),
                    source: *source,
                    imported: imported.map(Into::into),
                    type_only: false,
                })
                .collect(),
            stars: stars.iter().map(|star| (*star, false)).collect(),
        }
    }

    /// A module of code that exports `names`.
    fn module(names: &[&str]) -> Exports<&'static str> {
        Exports {
            names: names.iter().map(|name| ImportedName::new(*name, false)).collect(),
            ..Exports::default()
        }
    }

    /// An import of `names` by `importer` from `imported`.
    fn import<'a>(importer: &'a str, imported: &'a str, names: &'a [&'a str]) -> Import<'a> {
        Import { importer, imported, names }
    }

    #[test]
    fn named_imports_go_to_the_modules_that_define_them() {
        let (graph, exports) = project(
            &[
                import("app.ts", "lib/index.ts", &["format", "Client"]),
                import("lib/index.ts", "lib/format.ts", &["format"]),
                import("lib/index.ts", "lib/client.ts", &["Client"]),
                import("lib/index.ts", "lib/server.ts", &["*"]),
                import("lib/server.ts", "npm:sharp", &["*"]),
            ],
            &[
                (
                    "lib/index.ts",
                    barrel(
                        &[
                            ("format", "lib/format.ts", Some("format")),
                            ("Client", "lib/client.ts", Some("Client")),
                        ],
                        &["lib/server.ts"],
                    ),
                ),
                ("lib/format.ts", module(&["format"])),
                ("lib/client.ts", module(&["Client"])),
                ("lib/server.ts", module(&["serve"])),
            ],
        );
        // The app no longer reaches the server code, or `sharp`, through the barrel.
        assert_eq!(
            imports(&graph.through_reexports(&exports)),
            [
                "app.ts -> lib/client.ts",
                "app.ts -> lib/format.ts",
                "lib/index.ts -> lib/client.ts",
                "lib/index.ts -> lib/format.ts",
                "lib/index.ts -> lib/server.ts",
                "lib/server.ts -> npm:sharp",
            ]
        );
        // The graph itself is untouched.
        assert!(imports(&graph).contains(&"app.ts -> lib/index.ts".to_string()));
    }

    #[test]
    fn names_are_followed_through_reexports_of_barrels_and_export_star() {
        let (graph, exports) = project(
            &[
                import("app.ts", "index.ts", &["a", "b", "default", "ns"]),
                import("index.ts", "inner/index.ts", &["*"]),
                import("index.ts", "c.ts", &["c"]),
                import("index.ts", "ns.ts", &["*"]),
                import("inner/index.ts", "inner/a.ts", &["*"]),
                import("inner/index.ts", "inner/b.ts", &["original"]),
            ],
            &[
                (
                    "index.ts",
                    barrel(
                        &[("default", "c.ts", Some("c")), ("ns", "ns.ts", None)],
                        &["inner/index.ts"],
                    ),
                ),
                (
                    "inner/index.ts",
                    barrel(&[("b", "inner/b.ts", Some("original"))], &["inner/a.ts"]),
                ),
                ("inner/a.ts", module(&["a"])),
                ("inner/b.ts", module(&["original"])),
                ("c.ts", module(&["c"])),
                ("ns.ts", module(&["x"])),
            ],
        );
        let through = graph.through_reexports(&exports);
        let app: Vec<_> =
            imports(&through).into_iter().filter(|import| import.starts_with("app.ts")).collect();
        assert_eq!(
            app,
            ["app.ts -> c.ts", "app.ts -> inner/a.ts", "app.ts -> inner/b.ts", "app.ts -> ns.ts"]
        );
    }

    #[test]
    fn whole_modules_and_unknown_names_stay_on_the_barrel() {
        let (graph, exports) = project(
            &[
                import("star.ts", "index.ts", &["*"]),
                import("missing.ts", "index.ts", &["nope"]),
                import("external.ts", "pkg.ts", &["thing"]),
                import("index.ts", "a.ts", &["a"]),
                import("pkg.ts", "npm:lib", &["*"]),
            ],
            &[
                ("index.ts", barrel(&[("a", "a.ts", Some("a"))], &[])),
                // `thing` may come from the external package, which can't be told.
                ("pkg.ts", barrel(&[], &["npm:lib"])),
                ("a.ts", module(&["a"])),
            ],
        );
        let through = graph.through_reexports(&exports);
        let stayed: Vec<_> = imports(&through)
            .into_iter()
            .filter(|import| !import.starts_with("index.ts") && !import.starts_with("pkg.ts"))
            .collect();
        assert_eq!(
            stayed,
            ["external.ts -> pkg.ts", "missing.ts -> index.ts", "star.ts -> index.ts"]
        );
    }

    #[test]
    fn types_only_imports_of_names_become_type_imports() {
        let mut types = barrel(&[("Props", "props.ts", Some("Props"))], &[]);
        types.reexports.push(ReExport {
            name: "Theme".into(),
            source: "theme.ts",
            imported: Some("Theme".into()),
            type_only: true,
        });
        let (graph, exports) = project(
            &[
                import("app.ts", "index.ts", &["Props:type", "Theme", "render"]),
                import("index.ts", "props.ts", &["Props"]),
                import("index.ts", "theme.ts", &["Theme"]),
                import("index.ts", "render.ts", &["*"]),
            ],
            &[
                ("index.ts", Exports { stars: thin_vec![("render.ts", false)], ..types }),
                ("props.ts", module(&["Props"])),
                ("theme.ts", module(&["Theme"])),
                ("render.ts", module(&["render"])),
            ],
        );
        let through = graph.through_reexports(&exports);
        let app: Vec<_> =
            imports(&through).into_iter().filter(|import| import.starts_with("app.ts")).collect();
        assert_eq!(
            app,
            ["app.ts -> props.ts (type)", "app.ts -> render.ts", "app.ts -> theme.ts (type)"]
        );
    }

    #[test]
    fn cycles_of_re_exports_end() {
        let (graph, exports) = project(
            &[
                import("app.ts", "a/index.ts", &["x", "y"]),
                import("a/index.ts", "b/index.ts", &["*"]),
                import("b/index.ts", "a/index.ts", &["*"]),
                import("b/index.ts", "y.ts", &["y"]),
            ],
            &[
                ("a/index.ts", barrel(&[], &["b/index.ts"])),
                ("b/index.ts", barrel(&[("y", "y.ts", Some("y"))], &["a/index.ts"])),
                ("y.ts", module(&["y"])),
            ],
        );
        let through = graph.through_reexports(&exports);
        let app: Vec<_> =
            imports(&through).into_iter().filter(|import| import.starts_with("app.ts")).collect();
        // `x` is nowhere, so its import stays on the barrel.
        assert_eq!(app, ["app.ts -> a/index.ts", "app.ts -> y.ts"]);
    }
}
