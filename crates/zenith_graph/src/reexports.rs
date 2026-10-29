//! The import graph through re-exports: an import of names from a module
//! that only re-exports other modules, a pass-through module, becomes imports
//! of the modules that define those names.
//!
//! Bundlers that know a pass-through module has no side effects leave out the
//! modules it re-exports that nothing uses, so `import { a } from "./index"`
//! only loads the module that defines `a`. Through the pass-through module, the
//! import graph has a chain to every module it re-exports, so contracts about
//! what code reaches what, such as "client code must not reach server code",
//! report violations that the bundle doesn't have. The graph through re-exports
//! has the same modules, and imports to where each name is defined instead.

use compact_str::CompactString;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{Exports, Graph, ImportDetail, ImportedNames, ModuleId};

/// What each module exports, by module, with the source of each re-export
/// as the node it resolved to. A module without an entry is taken to be code
/// whose exports are unknown, such as an external package.
pub type ModuleExports = FxHashMap<ModuleId, Exports<ModuleId>>;

impl Graph {
    /// This graph, with each import of names from a pass-through module
    /// replaced by imports of the modules that define them, following
    /// re-exports and `export *` through nested pass-through modules. A name is
    /// defined by the first module on its way that isn't a pass-through
    /// module.
    ///
    /// An import of a pass-through module that takes the whole module, such as
    /// a namespace import, stays, and so does an import of a name that
    /// can't be followed: one the pass-through module doesn't export, or
    /// that may come from an external package through `export *`. An import
    /// of only a name's type becomes a type import.
    pub fn through_reexports(&self, exports: &ModuleExports) -> Graph {
        // Each importer's imports are followed apart from the others'.
        let names = Names { exports };
        let imports = (0..self.node_count())
            .into_par_iter()
            .map(ModuleId::new)
            .flat_map_iter(|importer| {
                let mut imports = Vec::new();
                for &imported in self.imports_of(importer) {
                    names.follow_imports(self, importer, imported, &mut imports);
                }
                imports
            })
            .collect();
        self.with_imports(imports)
    }
}

/// The `(pass-through module, name)` pairs met while following one name: `None`
/// while the pair is being followed, then the modules that define the name as
/// the pass-through module exports it, each with whether only its type gets
/// there.
type Followed = FxHashMap<(ModuleId, CompactString), Option<Vec<(ModuleId, bool)>>>;

/// Follows names through the exports of modules.
struct Names<'e> {
    /// What each module exports.
    exports: &'e ModuleExports,
}

impl Names<'_> {
    /// Add the imports of `imported` by `importer`, in `graph`, to `imports`,
    /// each import of names from a pass-through module replaced by imports of
    /// the modules that define them.
    fn follow_imports(
        &self,
        graph: &Graph,
        importer: ModuleId,
        imported: ModuleId,
        imports: &mut Vec<(ModuleId, ModuleId, ImportDetail)>,
    ) {
        let exports = self.exports;
        let pass_through = exports.get(&imported).is_some_and(|exports| exports.pass_through);
        for detail in graph.details(importer, imported) {
            let ImportedNames::Names(taken) = &detail.names else {
                imports.push((importer, imported, detail.clone()));
                continue;
            };
            if !pass_through {
                imports.push((importer, imported, detail.clone()));
                continue;
            }

            // Where each name goes: to the modules that define it, or to the
            // pass-through module if it can't be followed. A pass-through module may
            // re-export the importer's own names.
            let mut targets = Vec::new();
            for name in taken {
                match self.definitions(imported, &name.name, name.type_only) {
                    Some(definitions) => targets.extend(definitions),
                    None => targets.push((imported, name.type_only)),
                }
            }
            targets.retain(|(target, _)| *target != importer);

            // One detail per module that the statement imports, a type import
            // only if every name that goes there is a type.
            targets.sort_unstable();
            targets.dedup_by_key(|(target, _)| *target);
            for (target, type_only) in targets {
                let detail = match target == imported {
                    true => detail.clone(),
                    false => detail.with_type_only(type_only),
                };
                imports.push((importer, target, detail));
            }
        }
    }

    /// The modules that define `name` as `module` exports it, each with
    /// whether only its type gets there, or `None` if they can't be told.
    /// `type_only` is whether only its type is wanted.
    fn definitions(
        &self,
        module: ModuleId,
        name: &str,
        type_only: bool,
    ) -> Option<Vec<(ModuleId, bool)>> {
        let definitions = self.follow(module, name, &mut Followed::default())?;
        let definitions = definitions
            .into_iter()
            .map(|(definition, type_only_there)| (definition, type_only || type_only_there));
        let definitions: Vec<_> = definitions.collect();
        (!definitions.is_empty()).then_some(definitions)
    }

    /// The modules that define `name` as `module` exports it, each with
    /// whether only its type gets there, none if it doesn't export it, or
    /// `None` if that can't be told.
    fn follow(
        &self,
        module: ModuleId,
        name: &str,
        followed: &mut Followed,
    ) -> Option<Vec<(ModuleId, bool)>> {
        // 1. A module of code defines what it exports, and so does a pass-through
        //    module what it defines itself, as a Rust module may.
        let Some(exports) = self.exports.get(&module).filter(|exports| exports.pass_through) else {
            return Some(vec![(module, false)]);
        };
        if exports.names.iter().any(|defined| defined.name == name) {
            return Some(vec![(module, false)]);
        }

        // 2. A pair met before has the definitions found then, as when two pass-through
        //    modules re-export a third. One that is still being followed, which a cycle
        //    of re-exports comes back to, adds none: those on the rest of the cycle are
        //    found where it started.
        let key = (module, CompactString::from(name));
        match followed.get(&key) {
            Some(Some(definitions)) => return Some(definitions.clone()),
            Some(None) => return Some(Vec::new()),
            None => {}
        }
        followed.insert(key.clone(), None);

        // 3. A name the pass-through module re-exports by name comes from its source:
        //    the whole source, for `export * as name`, or for a name that the source
        //    doesn't export, which is the source itself, as Rust's `pub use a::b`
        //    re-exports the module `b`.
        let definitions = match exports.reexports.iter().find(|reexport| reexport.name == name) {
            Some(reexport) => match &reexport.imported {
                Some(imported)
                    if self.exports_name(reexport.source, imported, &mut FxHashSet::default())
                        != Some(false) =>
                {
                    let definitions = self.follow(reexport.source, imported, followed)?;
                    with_type_only(definitions, reexport.type_only)
                }
                _ => vec![(reexport.source, reexport.type_only)],
            },

            // 4. Any other name, but `default`, may come through `export *` from the modules that
            //    export it. If one of them may, but can't be told to, the name can't be followed.
            None if name == "default" => return None,
            None => {
                let mut definitions = Vec::new();
                for (star, star_type_only) in &exports.stars {
                    match self.exports_name(*star, name, &mut FxHashSet::default()) {
                        Some(true) => {
                            let found = self.follow(*star, name, followed)?;
                            definitions.extend(with_type_only(found, *star_type_only));
                        }
                        Some(false) => {}
                        None => return None,
                    }
                }
                definitions
            }
        };
        followed.insert(key, Some(definitions.clone()));
        Some(definitions)
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

/// `definitions`, with only their types getting there if `type_only`.
fn with_type_only(definitions: Vec<(ModuleId, bool)>, type_only: bool) -> Vec<(ModuleId, bool)> {
    let definitions = definitions.into_iter();
    definitions
        .map(|(definition, type_only_there)| (definition, type_only || type_only_there))
        .collect()
}

#[cfg(test)]
mod tests {
    use thin_vec::{ThinVec, thin_vec};
    use zenith_utils::location::ByteRange;

    use super::*;
    use crate::{GraphBuilder, ImportDetail, ImportKind, ImportedName, NodeHandle, ReExport};

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

    /// A pass-through module with these re-exports, as `(name, source,
    /// imported)`, and `export *` of the `stars`.
    fn pass_through(
        reexports: &[(&'static str, &'static str, Option<&str>)],
        stars: &[&'static str],
    ) -> Exports<&'static str> {
        Exports {
            pass_through: true,
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
                    pass_through(
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
        // The app no longer reaches the server code, or `sharp`, through the
        // pass-through module.
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
    fn names_are_followed_through_nested_re_exports_and_export_star() {
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
                    pass_through(
                        &[("default", "c.ts", Some("c")), ("ns", "ns.ts", None)],
                        &["inner/index.ts"],
                    ),
                ),
                (
                    "inner/index.ts",
                    pass_through(&[("b", "inner/b.ts", Some("original"))], &["inner/a.ts"]),
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
    fn a_pass_through_module_defines_what_it_defines_itself() {
        // As a Rust module does, which runs no code when imported, and may
        // glob-import a name that it shadows.
        let lib = Exports {
            names: thin_vec![ImportedName::new("Layout", false)],
            ..pass_through(&[], &["src/shapes.rs"])
        };
        let (graph, exports) = project(
            &[
                import("src/typeset.rs", "src/lib.rs", &["Layout"]),
                import("src/lib.rs", "src/shapes.rs", &["*"]),
            ],
            &[("src/lib.rs", lib), ("src/shapes.rs", module(&["Layout"]))],
        );
        assert_eq!(
            imports(&graph.through_reexports(&exports)),
            ["src/lib.rs -> src/shapes.rs", "src/typeset.rs -> src/lib.rs"]
        );
    }

    #[test]
    fn a_name_that_its_source_does_not_export_is_the_source() {
        // Rust's `pub use document;` re-exports the module `document` by its name.
        let (graph, exports) = project(
            &[
                import("src/app.rs", "src/lib.rs", &["document"]),
                import("src/lib.rs", "src/document.rs", &["document"]),
            ],
            &[
                (
                    "src/lib.rs",
                    pass_through(&[("document", "src/document.rs", Some("document"))], &[]),
                ),
                ("src/document.rs", module(&["Document"])),
            ],
        );
        assert_eq!(
            imports(&graph.through_reexports(&exports)),
            ["src/app.rs -> src/document.rs", "src/lib.rs -> src/document.rs"]
        );
    }

    #[test]
    fn whole_modules_and_unknown_names_stay_on_the_importer() {
        let (graph, exports) = project(
            &[
                import("star.ts", "index.ts", &["*"]),
                import("missing.ts", "index.ts", &["nope"]),
                import("external.ts", "pkg.ts", &["thing"]),
                import("index.ts", "a.ts", &["a"]),
                import("pkg.ts", "npm:lib", &["*"]),
            ],
            &[
                ("index.ts", pass_through(&[("a", "a.ts", Some("a"))], &[])),
                // `thing` may come from the external package, which can't be told.
                ("pkg.ts", pass_through(&[], &["npm:lib"])),
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
        let mut types = pass_through(&[("Props", "props.ts", Some("Props"))], &[]);
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
                ("a/index.ts", pass_through(&[], &["b/index.ts"])),
                ("b/index.ts", pass_through(&[("y", "y.ts", Some("y"))], &["a/index.ts"])),
                ("y.ts", module(&["y"])),
            ],
        );
        let through = graph.through_reexports(&exports);
        let app: Vec<_> =
            imports(&through).into_iter().filter(|import| import.starts_with("app.ts")).collect();
        // `x` is nowhere, so its import stays on the pass-through module.
        assert_eq!(app, ["app.ts -> a/index.ts", "app.ts -> y.ts"]);
    }

    /// The imports of `importer` in `graph`, as [`imports`] has them.
    fn imports_of(graph: &Graph, importer: &str) -> Vec<String> {
        let prefix = format!("{importer} -> ");
        imports(graph).into_iter().filter(|import| import.starts_with(&prefix)).collect()
    }

    #[test]
    fn names_are_followed_through_modules_that_share_a_re_export() {
        let (graph, exports) = project(
            &[
                import("app.ts", "ui/index.ts", &["t"]),
                import("ui/index.ts", "buttons/index.ts", &["*"]),
                import("ui/index.ts", "forms/index.ts", &["*"]),
                import("buttons/index.ts", "tokens/index.ts", &["*"]),
                import("forms/index.ts", "tokens/index.ts", &["*"]),
                import("tokens/index.ts", "tokens/t.ts", &["t"]),
            ],
            &[
                ("ui/index.ts", pass_through(&[], &["buttons/index.ts", "forms/index.ts"])),
                ("buttons/index.ts", pass_through(&[], &["tokens/index.ts"])),
                ("forms/index.ts", pass_through(&[], &["tokens/index.ts"])),
                ("tokens/index.ts", pass_through(&[("t", "tokens/t.ts", Some("t"))], &[])),
                ("tokens/t.ts", module(&["t"])),
            ],
        );
        let through = graph.through_reexports(&exports);
        assert_eq!(imports_of(&through, "app.ts"), ["app.ts -> tokens/t.ts"]);
        assert_eq!(
            through
                .details(through.id("app.ts").unwrap(), through.id("tokens/t.ts").unwrap())
                .len(),
            1
        );
    }

    #[test]
    fn names_are_followed_out_of_cycles_of_export_star() {
        let (graph, exports) = project(
            &[
                import("app.ts", "a/index.ts", &["z"]),
                import("main.ts", "b/index.ts", &["z"]),
                import("a/index.ts", "b/index.ts", &["*"]),
                import("a/index.ts", "c.ts", &["*"]),
                import("b/index.ts", "a/index.ts", &["*"]),
            ],
            &[
                ("a/index.ts", pass_through(&[], &["b/index.ts", "c.ts"])),
                ("b/index.ts", pass_through(&[], &["a/index.ts"])),
                ("c.ts", module(&["z"])),
            ],
        );
        let through = graph.through_reexports(&exports);
        assert_eq!(imports_of(&through, "app.ts"), ["app.ts -> c.ts"]);
        assert_eq!(imports_of(&through, "main.ts"), ["main.ts -> c.ts"]);
    }

    #[test]
    fn imports_of_the_importers_own_names_go_nowhere() {
        let (graph, exports) = project(
            &[
                import("a.ts", "index.ts", &["a", "b"]),
                import("index.ts", "a.ts", &["a"]),
                import("index.ts", "b.ts", &["b"]),
            ],
            &[
                (
                    "index.ts",
                    pass_through(&[("a", "a.ts", Some("a")), ("b", "b.ts", Some("b"))], &[]),
                ),
                ("a.ts", module(&["a"])),
                ("b.ts", module(&["b"])),
            ],
        );
        let through = graph.through_reexports(&exports);
        assert_eq!(imports_of(&through, "a.ts"), ["a.ts -> b.ts"]);
    }

    #[test]
    fn a_statement_imports_each_module_once() {
        // `A` and `B` both come from `m.ts`, `B` as a type; `x` is nowhere; `T`
        // comes through `export type *`.
        let mut index = pass_through(&[("A", "m.ts", Some("A"))], &[]);
        index.reexports.push(ReExport {
            name: "B".into(),
            source: "m.ts",
            imported: Some("B".into()),
            type_only: true,
        });
        index.stars.push(("types.ts", true));
        let (graph, exports) = project(
            &[
                import("app.ts", "index.ts", &["A", "B", "x", "T"]),
                import("index.ts", "m.ts", &["A", "B"]),
                import("index.ts", "types.ts", &["*"]),
            ],
            &[("index.ts", index), ("m.ts", module(&["A", "B"])), ("types.ts", module(&["T"]))],
        );
        let through = graph.through_reexports(&exports);
        assert_eq!(
            imports_of(&through, "app.ts"),
            ["app.ts -> index.ts", "app.ts -> m.ts", "app.ts -> types.ts (type)"]
        );
        let (app, m) = (through.id("app.ts").unwrap(), through.id("m.ts").unwrap());
        let kinds: Vec<_> = through.details(app, m).iter().map(|detail| detail.kind).collect();
        assert_eq!(kinds, [ImportKind::Value]);
    }
}
