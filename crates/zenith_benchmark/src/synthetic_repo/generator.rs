//! Generating the contents of a planned synthetic repository: each source
//! file's imports and body, and the manifests, configs and skipped files
//! around them.

use std::collections::BTreeSet;

use fastrand::Rng;
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use zenith_graph::ImportKind;
use zenith_utils::{path::posix_parent, specifier::package_name};

use super::{
    FileKind, ImportTarget, PackageRole, SyntheticFile, SyntheticImport,
    plan::{Category, Feature, Module, PackagePlan, file_name},
    templates::*,
};

/// The npm packages that the repository imports: each one's name, version
/// range, and the subpaths that are imported from it.
const EXTERNALS: &[(&str, &str, &[&str])] = &[
    ("@hono/node-server", "^1.14.4", &[]),
    ("@tanstack/react-query", "^5.83.0", &[]),
    ("bullmq", "^5.56.4", &[]),
    ("clsx", "^2.1.1", &[]),
    ("date-fns", "^4.1.0", &["locale"]),
    ("drizzle-orm", "^0.44.3", &["pg-core"]),
    ("hono", "^4.8.5", &[]),
    ("ky", "^1.8.2", &[]),
    ("lodash-es", "^4.17.21", &["debounce"]),
    ("nanoid", "^5.1.5", &[]),
    ("next", "^15.4.2", &["link", "navigation"]),
    ("react", "^19.1.0", &[]),
    ("react-dom", "^19.1.0", &["client"]),
    ("vitest", "^3.2.4", &[]),
    ("zod", "^4.0.5", &[]),
];

/// The npm imports that a utility module picks from: the import clause and
/// the specifier.
const UTIL_EXTERNALS: &[(&str, &str)] = &[
    ("{ format }", "date-fns"),
    ("{ enGB }", "date-fns/locale"),
    ("debounce", "lodash-es/debounce"),
    ("{ nanoid }", "nanoid"),
];

impl PackagePlan {
    /// Generate the package's files: its source files, then the rest. Also
    /// returns the npm packages that its source files import.
    pub(super) fn generate(
        &self,
        rng: &mut Rng,
        plans: &[PackagePlan],
    ) -> (Vec<SyntheticFile>, BTreeSet<&'static str>) {
        let mut generator = Generator { rng, plans, plan: self, externals: BTreeSet::new() };
        let mut files: Vec<SyntheticFile> =
            (0..self.modules.len()).map(|module| generator.module(module)).collect();

        let externals = generator.externals;
        files.extend(self.non_source_files(plans, &externals));
        (files, externals)
    }

    /// The files of the package other than its source files: its manifest,
    /// `tsconfig.json` and README, an app's assets, and the build output and
    /// generated code that the walk must skip.
    fn non_source_files(
        &self,
        plans: &[PackagePlan],
        externals: &BTreeSet<&str>,
    ) -> Vec<SyntheticFile> {
        let package = &self.package;
        let dir = &package.dir;
        let file = |path: &str, contents: &str, kind| {
            SyntheticFile::new(format!("{dir}/{path}"), contents.to_string(), kind)
        };
        let readme = format!("# {}\n\nPart of the synthetic acme monorepo.\n", package.name);
        let mut files = vec![
            file("package.json", &self.package_json(plans, externals), FileKind::Manifest),
            file("tsconfig.json", &package.role.tsconfig_json(), FileKind::Other),
            file("README.md", &readme, FileKind::Other),
        ];

        match package.role {
            PackageRole::App => files.extend([
                file(
                    "src/styles/globals.css",
                    ":root {\n  color-scheme: light dark;\n}\n",
                    FileKind::Other,
                ),
                file(
                    "src/assets/logo.svg",
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1 1\"/>\n",
                    FileKind::Other,
                ),
                file(".next/server/app/page.js", "\"use strict\";\n", FileKind::Skipped),
                file(".next/build-manifest.json", "{}\n", FileKind::Skipped),
            ]),
            PackageRole::Library if package.has_exports => files.extend([
                file("dist/index.js", "export {};\n", FileKind::Skipped),
                file("dist/index.d.ts", "export {};\n", FileKind::Skipped),
            ]),
            PackageRole::Library => files.extend([
                file("src/__generated__/graphql.ts", GENERATED, FileKind::Skipped),
                file("src/__generated__/schema.ts", GENERATED, FileKind::Skipped),
            ]),
            PackageRole::Service => {}
        }
        files
    }

    /// The package's `package.json`.
    fn package_json(&self, plans: &[PackagePlan], externals: &BTreeSet<&str>) -> String {
        // 1. The name, and how the package is imported.
        let package = &self.package;
        let mut json = format!(
            "{{\n  \"name\": \"{}\",\n  \"version\": \"0.0.0\",\n  \"private\": true,\n  \
             \"type\": \"module\",\n",
            package.name
        );
        match package.role {
            PackageRole::Library if package.has_exports => json.push_str(EXPORTS_MAP),
            PackageRole::Library => {
                json.push_str(
                    "  \"main\": \"./src/index.ts\",\n  \"types\": \"./src/index.ts\",\n",
                );
            }
            _ => {}
        }

        // 2. The scripts of its role.
        json.push_str(match package.role {
            PackageRole::Library => {
                "  \"scripts\": {\n    \"build\": \"tsc -p tsconfig.json\",\n    \"test\": \
                 \"vitest run\"\n  },\n"
            }
            PackageRole::Service => {
                "  \"scripts\": {\n    \"dev\": \"tsx watch src/main.ts\",\n    \"start\": \"node \
                 dist/main.js\",\n    \"test\": \"vitest run\"\n  },\n"
            }
            PackageRole::App => {
                "  \"scripts\": {\n    \"dev\": \"next dev\",\n    \"build\": \"next build\",\n    \
                 \"test\": \"vitest run\"\n  },\n"
            }
        });

        // 3. Its workspace and npm dependencies, sorted by name.
        let mut dependencies: Vec<(&str, &str)> = self
            .dependencies
            .iter()
            .map(|&dependency| (plans[dependency].package.name.as_str(), "workspace:*"))
            .collect();
        dependencies.extend(externals.iter().map(|&name| (name, external_version(name))));
        dependencies.sort_unstable();
        let entries: Vec<String> = dependencies
            .iter()
            .map(|(name, version)| format!("    \"{name}\": \"{version}\""))
            .collect();
        json.push_str(&format!("  \"dependencies\": {{\n{}\n  }}\n}}\n", entries.join(",\n")));
        json
    }

    /// Finish a path specifier without an extension: services, written as
    /// Node.js ES modules, add `.js`, and bundled packages import a
    /// directory's `index` by the directory.
    fn finish_specifier(&self, path: String) -> String {
        if self.package.role == PackageRole::Service {
            format!("{path}.js")
        } else {
            match path.strip_suffix("/index") {
                Some(dir) => dir.to_string(),
                None => path,
            }
        }
    }
}

/// Generates the source files of one package.
struct Generator<'a> {
    /// The repository's random numbers.
    rng: &'a mut Rng,

    /// Every package of the repository.
    plans: &'a [PackagePlan],

    /// The package being generated.
    plan: &'a PackagePlan,

    /// The npm packages that the package's files import so far.
    externals: BTreeSet<&'static str>,
}

impl<'a> Generator<'a> {
    /// Generate one of the package's source files.
    fn module(&mut self, index: usize) -> SyntheticFile {
        let plan = self.plan;
        let module = &plan.modules[index];
        let mut source = Source::default();
        match module.category {
            Category::PackageBarrel => self.package_barrel(&mut source),
            Category::Server => self.server(&mut source, module),
            Category::AppEntry => self.app_entry(&mut source),
            Category::ServiceEntry => self.service_entry(&mut source),
            Category::Script => self.script(&mut source),
            Category::FeatureBarrel => self.feature_barrel(&mut source, module),
            Category::Types => {
                let feature = self.feature_of(module).name.to_upper_camel_case();
                source.body = TYPES.replace("__P__", &feature);
            }
            Category::Route => self.route(&mut source, module),
            Category::Test => self.test(&mut source, module),
            _ => self.importable(&mut source, index),
        }

        let (contents, imports) = source.finish();
        SyntheticFile { path: plan.path(module), contents, kind: FileKind::Source, imports }
    }

    /// The feature of a feature module.
    fn feature_of(&self, module: &Module) -> &'a Feature {
        &self.plan.features
            [module.feature.expect("only feature modules are asked for their feature")]
    }

    /// Import from an npm package.
    fn external(&mut self, source: &mut Source, clause: &str, specifier: &'static str) {
        self.externals.insert(package_name(specifier).expect("npm specifiers name a package"));
        source.import(clause, specifier, To::EXTERNAL, ImportKind::Value);
    }

    /// The relative specifier for a module of the package from a file in
    /// `from_dir`, relative to the package, and where it points.
    fn relative(&self, from_dir: &str, module: &Module) -> (String, To) {
        let specifier = relative_specifier(from_dir, &module.stem);
        let to = To::file(ImportTarget::Relative, self.plan.path(module));
        (self.plan.finish_specifier(specifier), to)
    }

    /// The `@/` alias specifier for a module of the package, and where it
    /// points.
    fn alias(&self, module: &Module) -> (String, To) {
        let stem = &module.stem;
        let specifier = format!("@/{}", stem.strip_prefix("src/").unwrap_or(stem));
        let to = To::file(ImportTarget::Alias, self.plan.path(module));
        (self.plan.finish_specifier(specifier), to)
    }

    /// The import clause for a module's symbol, as a value or as a type.
    fn clause(&mut self, module: &Module, type_only: bool) -> String {
        match module.category {
            Category::Component if type_only => format!("{{ {}Props }}", module.symbol),
            Category::Component if self.rng.f32() < 0.15 => {
                format!("{{ {0}, type {0}Props }}", module.symbol)
            }
            _ => format!("{{ {} }}", module.symbol),
        }
    }

    /// A library's barrel: it re-exports every top-level feature, some as
    /// namespaces.
    fn package_barrel(&mut self, source: &mut Source) {
        let plan = self.plan;
        for feature in plan.features.iter().filter(|feature| feature.parent.is_none()) {
            let barrel = &plan.modules[feature.modules[0]];
            let (specifier, to) = self.relative("src", barrel);
            let clause = if feature.namespaced {
                format!("* as {}", feature.name.to_lower_camel_case())
            } else {
                "*".into()
            };
            source.reexport(&clause, &specifier, to, ImportKind::Value);
        }
    }

    /// A feature's barrel: it re-exports the feature's modules and nested
    /// features.
    fn feature_barrel(&mut self, source: &mut Source, barrel: &Module) {
        let plan = self.plan;
        let feature = self.feature_of(barrel);
        let dir = &feature.dir;
        for &index in &feature.modules[1..] {
            let module = &plan.modules[index];
            let (specifier, to) = self.relative(dir, module);
            let (clause, kind) = match module.category {
                Category::Types => ("*".to_string(), ImportKind::Type),
                Category::Component => {
                    (format!("{{ {0}, type {0}Props }}", module.symbol), ImportKind::Value)
                }
                category if category.is_importable() => ("*".to_string(), ImportKind::Value),
                _ => continue,
            };
            source.reexport(&clause, &specifier, to, kind);
        }

        let index = barrel.feature.expect("a feature barrel belongs to its feature");
        for child in plan.features.iter().filter(|child| child.parent == Some(index)) {
            let (specifier, to) = self.relative(dir, &plan.modules[child.modules[0]]);
            source.reexport("*", &specifier, to, ImportKind::Value);
        }
    }

    /// A component, hook, client, utility, handler, job or store.
    fn importable(&mut self, source: &mut Source, index: usize) {
        let plan = self.plan;
        let module = &plan.modules[index];
        let feature = self.feature_of(module);
        let dir = posix_parent(&module.stem);

        // 1. Import npm packages, builtins, other packages and other features.
        self.category_imports(source, module.category);
        self.workspace_imports(source);
        self.alias_imports(source, module);

        // 2. Import siblings in the same feature, and its types.
        let siblings: Vec<usize> = plan
            .importables(module.feature.unwrap_or_default())
            .into_iter()
            .filter(|&i| i != index)
            .collect();
        let wanted = self.rng.usize(..3)
            + usize::from(matches!(
                module.category,
                Category::Component | Category::Hook | Category::Handler | Category::Job
            ));
        for _ in 0..wanted.min(siblings.len()) {
            let target = &plan.modules[*self.rng.choice(&siblings).expect("there are siblings")];
            let clause = self.clause(target, false);
            let (specifier, to) = self.relative(dir, target);
            source.import(&clause, &specifier, to, ImportKind::Value);
        }
        let mut modules = feature.modules.iter().map(|&i| &plan.modules[i]);
        if let Some(types) = modules.find(|m| m.category == Category::Types)
            && self.rng.bool()
        {
            let (specifier, to) = self.relative(dir, types);
            let clause = format!("{{ {} }}", types.symbol);
            source.import(&clause, &specifier, to, ImportKind::Type);
        }

        // 3. Fill in the category's template.
        let title = module.symbol.to_upper_camel_case();
        let kebab = format!("{}-{}", feature.name, file_name(&module.stem));
        let number = self.rng.usize(1..=5).to_string();
        let mut body = match module.category {
            Category::Component => {
                let class = if source.specifiers.contains("clsx") {
                    format!("clsx(\"{kebab}\", expanded && \"is-expanded\")")
                } else {
                    format!("expanded ? \"{kebab} is-expanded\" : \"{kebab}\"")
                };
                COMPONENT.replace("__CLASS__", &class)
            }
            Category::Hook => HOOK.to_string(),
            Category::Client => CLIENT.to_string(),
            Category::Util => UTIL.to_string(),
            Category::Handler => HANDLER.to_string(),
            Category::Job => JOB.to_string(),
            _ => STORE.to_string(),
        }
        .replace("__S__", &module.symbol)
        .replace("__T__", &title)
        .replace("__K__", &kebab)
        .replace("__N__", &number);

        // 4. Have a few hooks preload a sibling, and pad the module with helpers.
        if module.category == Category::Hook
            && self.rng.f32() < 0.1
            && let Some(&target) = self.rng.choice(&siblings)
        {
            let (specifier, to) = self.relative(dir, &plan.modules[target]);
            let import = source.dynamic(&specifier, to);
            body.push_str(&format!(
                "\nexport async function preload{title}() {{\n  return {import};\n}}\n"
            ));
        }
        body.push_str(&self.helpers(&module.symbol));
        source.body = body;
    }

    /// The npm and builtin imports that a category always or often has.
    fn category_imports(&mut self, source: &mut Source, category: Category) {
        let service = self.plan.package.role == PackageRole::Service;
        match category {
            Category::Component => {
                self.external(source, "{ useMemo, useState }", "react");
                if self.rng.bool() {
                    self.external(source, "clsx", "clsx");
                }
            }
            Category::Hook => {
                self.external(source, "{ useEffect, useState }", "react");
                self.external(source, "{ useQuery }", "@tanstack/react-query");
            }
            Category::Client => {
                self.external(source, "ky", "ky");
                self.external(source, "{ z }", "zod");
            }
            Category::Util => {
                for _ in 0..self.rng.usize(1..=2) {
                    let &(clause, specifier) =
                        self.rng.choice(UTIL_EXTERNALS).expect("the list isn't empty");
                    self.external(source, clause, specifier);
                }
                if service && self.rng.f32() < 0.3 {
                    let clause = "{ EventEmitter }";
                    source.import(clause, "node:events", To::BUILTIN, ImportKind::Value);
                } else if !service && self.rng.f32() < 0.1 {
                    source.import("path", "node:path", To::BUILTIN, ImportKind::Value);
                }
            }
            Category::Handler => {
                self.external(source, "{ Hono }", "hono");
                self.external(source, "{ z }", "zod");
                if self.rng.f32() < 0.4 {
                    source.import("path", "node:path", To::BUILTIN, ImportKind::Value);
                }
            }
            Category::Job => {
                self.external(source, "{ Queue }", "bullmq");
                if self.rng.f32() < 0.7 {
                    let clause = "{ randomUUID }";
                    source.import(clause, "node:crypto", To::BUILTIN, ImportKind::Value);
                }
                // Builtins can be written without the `node:` prefix.
                if self.rng.f32() < 0.2 {
                    source.import("os", "os", To::BUILTIN, ImportKind::Value);
                }
            }
            Category::Store => {
                self.external(source, "{ eq }", "drizzle-orm");
                self.external(source, "{ pgTable, text }", "drizzle-orm/pg-core");
                if service && self.rng.bool() {
                    let clause = "{ readFile }";
                    let specifier = "node:fs/promises";
                    source.import(clause, specifier, To::BUILTIN, ImportKind::Value);
                }
            }
            _ => {}
        }
    }

    /// Imports from the libraries that the package depends on: by name,
    /// through an `exports` subpath, or by a deep path.
    fn workspace_imports(&mut self, source: &mut Source) {
        let plans = self.plans;
        let plan = self.plan;
        let wanted = match plan.package.role {
            PackageRole::Library => usize::from(self.rng.f32() < 0.35),
            PackageRole::Service | PackageRole::App => self.rng.usize(..3),
        };

        for _ in 0..wanted {
            // 1. Pick a module of a top-level feature of a dependency.
            let Some(&dependency) = self.rng.choice(&plan.dependencies) else { return };
            let dependency = &plans[dependency];
            let features: Vec<usize> = (0..dependency.features.len())
                .filter(|&f| {
                    dependency.features[f].parent.is_none() && !dependency.importables(f).is_empty()
                })
                .collect();
            let Some(&index) = self.rng.choice(&features) else { continue };
            let feature = &dependency.features[index];
            let importables = dependency.importables(index);
            let target =
                &dependency.modules[*self.rng.choice(&importables).expect("it has importables")];
            let type_only = self.rng.f32() < 0.2;

            // 2. Import it through a library's `./server` or feature subpath, by a deep
            //    path, or from the package's barrel.
            let name = &dependency.package.name;
            let barrel = &dependency.modules[0];
            let server = dependency.modules.iter().find(|m| m.category == Category::Server);
            let (specifier, clause, resolved) = match server {
                Some(server)
                    if plan.package.role == PackageRole::Service && self.rng.f32() < 0.25 =>
                {
                    (format!("{name}/server"), format!("{{ {} }}", server.symbol), server)
                }
                _ if dependency.package.has_exports && self.rng.f32() < 0.3 => {
                    let specifier = format!("{name}/features/{}", feature.name);
                    let feature_barrel = &dependency.modules[feature.modules[0]];
                    (specifier, self.clause(target, type_only), feature_barrel)
                }
                _ if !dependency.package.has_exports && self.rng.f32() < 0.3 => {
                    let path = plan.finish_specifier(target.stem.clone());
                    (format!("{name}/{path}"), self.clause(target, type_only), target)
                }
                _ if feature.namespaced => {
                    (name.clone(), format!("{{ {} }}", feature.name.to_lower_camel_case()), barrel)
                }
                _ => (name.clone(), self.clause(target, type_only), barrel),
            };
            let kind = ImportKind::of_static(type_only);
            let to = To::file(ImportTarget::Workspace, dependency.path(resolved));
            source.import(&clause, &specifier, to, kind);
        }
    }

    /// Imports from the package's other features through the `@/` alias,
    /// from a feature's barrel or from a module directly.
    fn alias_imports(&mut self, source: &mut Source, module: &Module) {
        let plan = self.plan;
        let wanted = match self.rng.usize(..10) {
            0..=3 => 0,
            4..=7 => 1,
            _ => 2,
        };
        let others: Vec<usize> =
            (0..plan.features.len()).filter(|&f| Some(f) != module.feature).collect();
        for _ in 0..wanted {
            let Some(&feature) = self.rng.choice(&others) else { return };
            let Some(&target) = self.rng.choice(&plan.importables(feature)) else { continue };
            let target = &plan.modules[target];
            let type_only = self.rng.f32() < 0.2;
            let (specifier, to) = if self.rng.bool() {
                self.alias(&plan.modules[plan.features[feature].modules[0]])
            } else {
                self.alias(target)
            };
            let clause = self.clause(target, type_only);
            let kind = ImportKind::of_static(type_only);
            source.import(&clause, &specifier, to, kind);
        }
    }

    /// A library's `src/server.ts`.
    fn server(&mut self, source: &mut Source, module: &Module) {
        let plan = self.plan;
        source.import("{ readFile }", "node:fs/promises", To::BUILTIN, ImportKind::Value);
        source.import("path", "node:path", To::BUILTIN, ImportKind::Value);

        let importables: Vec<usize> =
            (0..plan.features.len()).flat_map(|feature| plan.importables(feature)).collect();
        let mut exported = Vec::new();
        for _ in 0..self.rng.usize(1..=2).min(importables.len()) {
            let target =
                &plan.modules[*self.rng.choice(&importables).expect("there are importables")];
            let (specifier, to) = self.relative("src", target);
            let clause = format!("{{ {} }}", target.symbol);
            source.import(&clause, &specifier, to, ImportKind::Value);
            exported.push(target.symbol.as_str());
        }
        exported.dedup();

        source.body = format!(
            "export async function {symbol}(dir: string): Promise<Record<string, unknown>> {{\n  \
             const contents = await readFile(path.join(dir, \"{name}.config.json\"), \"utf8\");\n  \
             return JSON.parse(contents) as Record<string, unknown>;\n}}\n\nexport const \
             serverExports = [{exported}] as const;\n",
            symbol = module.symbol,
            name = plan.short_name(),
            exported = exported.join(", "),
        );
    }

    /// An app's `src/main.tsx`: it imports a stylesheet, an image and some of
    /// its features.
    fn app_entry(&mut self, source: &mut Source) {
        let plan = self.plan;
        self.external(source, "{ createRoot }", "react-dom/client");
        for (clause, asset) in [("", "styles/globals.css"), ("logo", "assets/logo.svg")] {
            let to = To::file(ImportTarget::Relative, format!("{}/src/{asset}", plan.package.dir));
            source.import(clause, &format!("./{asset}"), to, ImportKind::Value);
        }

        let mut rendered = String::new();
        for feature in plan.features.iter().filter(|feature| feature.parent.is_none()).take(4) {
            let component = feature
                .modules
                .iter()
                .map(|&i| &plan.modules[i])
                .find(|m| m.category == Category::Component);
            if let Some(component) = component {
                let (specifier, to) = self.alias(&plan.modules[feature.modules[0]]);
                let clause = format!("{{ {} }}", component.symbol);
                source.import(&clause, &specifier, to, ImportKind::Value);
                rendered.push_str(&format!(
                    "      <{} id=\"{}\" items={{[]}} />\n",
                    component.symbol, feature.name
                ));
            }
        }

        source.body = format!(
            "const container = document.getElementById(\"root\");\n\nif (container) {{\n  \
             createRoot(container).render(\n    <>\n      <img src={{logo}} alt=\"Acme\" \
             />\n{rendered}    </>,\n  );\n}}\n"
        );
    }

    /// A service's `src/main.ts`: it mounts some of its handlers.
    fn service_entry(&mut self, source: &mut Source) {
        let plan = self.plan;
        self.external(source, "{ serve }", "@hono/node-server");
        self.external(source, "{ Hono }", "hono");
        source.import("{ env }", "node:process", To::BUILTIN, ImportKind::Value);

        let mut routes = String::new();
        for handler in plan.modules.iter().filter(|m| m.category == Category::Handler).take(4) {
            let (specifier, to) = self.relative("src", handler);
            let clause = format!("{{ {} }}", handler.symbol);
            source.import(&clause, &specifier, to, ImportKind::Value);
            routes.push_str(&format!("app.route(\"/\", {});\n", handler.symbol));
        }

        source.body = format!(
            "const app = new Hono();\n{routes}\nserve({{ fetch: app.fetch, port: \
             Number(env.PORT ?? 3000) }});\n"
        );
    }

    /// A service's CommonJS `scripts/seed.cjs`, which uses `require`.
    fn script(&mut self, source: &mut Source) {
        let fs = source.require("node:fs", To::BUILTIN);
        let path = source.require("node:path", To::BUILTIN);
        self.externals.insert("zod");
        let zod = source.require("zod", To::EXTERNAL);
        source.body = format!(
            "\"use strict\";\n\nconst {{ readFileSync }} = {fs};\nconst path = {path};\nconst {{ z \
             }} = {zod};\n\nconst Seed = z.object({{ name: z.string(), count: z.number() }});\n\nconst \
             file = path.join(__dirname, \"seed.json\");\nmodule.exports = \
             Seed.parse(JSON.parse(readFileSync(file, \"utf8\")));\n"
        );
    }

    /// An app route, which renders a component of its feature, sometimes
    /// loaded lazily.
    fn route(&mut self, source: &mut Source, module: &Module) {
        let plan = self.plan;
        let feature = self.feature_of(module);
        self.external(source, "Link", "next/link");
        self.external(source, "{ notFound }", "next/navigation");

        // 1. Pick a component of the feature, to import from its barrel or to load
        //    lazily.
        let components: Vec<&Module> = feature
            .modules
            .iter()
            .map(|&i| &plan.modules[i])
            .filter(|m| m.category == Category::Component)
            .collect();
        let component = self.rng.choice(&components).copied();
        let lazy = component.is_some() && self.rng.f32() < 0.4;
        if let Some(component) = component
            && !lazy
        {
            let (specifier, to) = self.alias(&plan.modules[feature.modules[0]]);
            let clause = format!("{{ {} }}", component.symbol);
            source.import(&clause, &specifier, to, ImportKind::Value);
        } else if lazy {
            self.external(source, "{ lazy }", "react");
        }

        // 2. Render it in the page.
        let mut body = String::new();
        let content = match component {
            Some(component) if lazy => {
                let (specifier, to) = self.alias(component);
                let import = source.dynamic(&specifier, to);
                body.push_str(&format!(
                    "const Lazy{0} = lazy(() =>\n  {import}.then((module) => ({{ default: \
                     module.{0} }})),\n);\n\n",
                    component.symbol
                ));
                format!(
                    "<Lazy{} id={{id ?? \"{}\"}} items={{[]}} />",
                    component.symbol, feature.name
                )
            }
            Some(component) => {
                format!("<{} id={{id ?? \"{}\"}} items={{[]}} />", component.symbol, feature.name)
            }
            None => format!("<h1>{}</h1>", feature.name),
        };
        body.push_str(&format!(
            "export default async function {symbol}({{ params }}: {{ params: Promise<{{ id?: string \
             }}> }}) {{\n  const {{ id }} = await params;\n  if (id === \"missing\") {{\n    \
             notFound();\n  }}\n\n  return (\n    <main>\n      <Link href=\"/{name}\">Back to \
             {name}</Link>\n      {content}\n    </main>\n  );\n}}\n",
            symbol = module.symbol,
            name = feature.name,
        ));
        source.body = body;
    }

    /// A test of another module in its feature.
    fn test(&mut self, source: &mut Source, module: &Module) {
        let plan = self.plan;
        let subject = &plan.modules[module.subject.expect("a test has a subject")];
        self.external(source, "{ describe, expect, it }", "vitest");
        let (specifier, to) = self.relative(posix_parent(&module.stem), subject);
        let clause = format!("{{ {} }}", subject.symbol);
        source.import(&clause, &specifier, to, ImportKind::Value);

        source.body =
            TEST.replace("__K__", file_name(&subject.stem)).replace("__SUBJECT__", &subject.symbol);
    }

    /// Up to three helper declarations, so that files vary in length.
    fn helpers(&mut self, symbol: &str) -> String {
        let mut helpers = String::new();
        for n in 0..self.rng.usize(..4) {
            let template = *self.rng.choice(HELPERS).expect("the list isn't empty");
            let name = format!("{}Helper{n}", symbol.to_lower_camel_case());
            let threshold = self.rng.usize(..100).to_string();
            helpers.push_str(
                &template
                    .replace("__H__", &name)
                    .replace("__E__", &name.to_upper_camel_case())
                    .replace("__N__", &threshold),
            );
        }
        helpers
    }
}

/// The text of a source file as it's built, and the imports in it.
#[derive(Default)]
struct Source {
    /// The import and re-export statements.
    header: String,

    /// Everything after them.
    body: String,

    /// The imports, in source order.
    imports: Vec<SyntheticImport>,

    /// The specifiers imported by the header, so that none is imported twice.
    specifiers: BTreeSet<String>,
}

impl Source {
    /// Add an import statement, unless the specifier is already imported. An
    /// empty clause is a side-effect import.
    fn import(&mut self, clause: &str, specifier: &str, to: To, kind: ImportKind) {
        if !self.specifiers.insert(specifier.to_string()) {
            return;
        }
        let keyword = if kind == ImportKind::Type { "import type" } else { "import" };
        if clause.is_empty() {
            self.header.push_str(&format!("{keyword} \"{specifier}\";\n"));
        } else {
            self.header.push_str(&format!("{keyword} {clause} from \"{specifier}\";\n"));
        }
        self.record(specifier, to, kind);
    }

    /// Add a re-export statement.
    fn reexport(&mut self, clause: &str, specifier: &str, to: To, kind: ImportKind) {
        let keyword = if kind == ImportKind::Type { "export type" } else { "export" };
        self.header.push_str(&format!("{keyword} {clause} from \"{specifier}\";\n"));
        self.record(specifier, to, kind);
    }

    /// A dynamic `import()` expression for the body.
    fn dynamic(&mut self, specifier: &str, to: To) -> String {
        self.record(specifier, to, ImportKind::Dynamic);
        format!("import(\"{specifier}\")")
    }

    /// A `require()` call for the body.
    fn require(&mut self, specifier: &str, to: To) -> String {
        self.record(specifier, to, ImportKind::Require);
        format!("require(\"{specifier}\")")
    }

    /// Record an import.
    fn record(&mut self, specifier: &str, to: To, kind: ImportKind) {
        self.imports.push(SyntheticImport {
            specifier: specifier.to_string(),
            target: to.target,
            kind,
            resolves_to: to.file,
        });
    }

    /// The file's contents and imports.
    fn finish(self) -> (String, Vec<SyntheticImport>) {
        let mut contents = self.header;
        if !self.body.is_empty() {
            if !contents.is_empty() {
                contents.push('\n');
            }
            contents.push_str(&self.body);
        }
        (contents, self.imports)
    }
}

/// What an import points at: the kind of target, and the file that it
/// resolves to, if it's in the repository.
struct To {
    /// The kind of target.
    target: ImportTarget,

    /// The file, as a POSIX path relative to the root.
    file: Option<String>,
}

impl To {
    /// An npm package.
    const EXTERNAL: Self = Self { target: ImportTarget::External, file: None };

    /// A Node.js builtin.
    const BUILTIN: Self = Self { target: ImportTarget::Builtin, file: None };

    /// A file of the repository.
    fn file(target: ImportTarget, path: String) -> Self {
        Self { target, file: Some(path) }
    }
}

impl PackageRole {
    /// The `tsconfig.json` of a package of this role, which extends the
    /// root's and adds the `@/` alias for its `src/`.
    fn tsconfig_json(self) -> String {
        let options = match self {
            Self::Library => "",
            Self::Service => {
                "    \"module\": \"NodeNext\",\n    \"moduleResolution\": \"NodeNext\",\n"
            }
            Self::App => "    \"jsx\": \"preserve\",\n    \"plugins\": [{ \"name\": \"next\" }],\n",
        };
        format!(
            "{{\n  \"extends\": \"../../tsconfig.json\",\n  \"compilerOptions\": {{\n{options}    \
             \"paths\": {{\n      \"@/*\": [\"./src/*\"]\n    }}\n  }},\n  \"include\": [\"src\"]\n}}\n"
        )
    }
}

impl SyntheticFile {
    /// A file that isn't a source file, so has no imports.
    fn new(path: String, contents: String, kind: FileKind) -> Self {
        Self { path, contents, kind, imports: Vec::new() }
    }
}

/// The files at the root: the workspace's manifest, `tsconfig.json`,
/// `.gitignore` and README, and stubs in `node_modules` for the npm packages.
pub(super) fn root_files(externals: &BTreeSet<&str>) -> Vec<SyntheticFile> {
    let mut files = vec![
        SyntheticFile::new("package.json".into(), ROOT_PACKAGE_JSON.into(), FileKind::Manifest),
        SyntheticFile::new("tsconfig.json".into(), ROOT_TSCONFIG_JSON.into(), FileKind::Other),
        SyntheticFile::new(".gitignore".into(), GITIGNORE.into(), FileKind::Other),
        SyntheticFile::new(
            "README.md".into(),
            "# acme\n\nA synthetic monorepo, generated for zenith's benchmarks.\n".into(),
            FileKind::Other,
        ),
    ];

    for &(name, version, subpaths) in EXTERNALS.iter().filter(|(name, ..)| externals.contains(name))
    {
        let dir = format!("node_modules/{name}");
        let version = version.trim_start_matches('^');
        let manifest = format!(
            "{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\",\n  \"type\": \"module\",\n  \
             \"exports\": {{\n    \".\": {{\n      \"types\": \"./index.d.ts\",\n      \"default\": \
             \"./index.js\"\n    }},\n    \"./*\": {{\n      \"types\": \"./*.d.ts\",\n      \
             \"default\": \"./*.js\"\n    }}\n  }}\n}}\n"
        );
        files.push(SyntheticFile::new(format!("{dir}/package.json"), manifest, FileKind::Skipped));
        for module in std::iter::once("index").chain(subpaths.iter().copied()) {
            files.extend(["js", "d.ts"].map(|extension| {
                SyntheticFile::new(
                    format!("{dir}/{module}.{extension}"),
                    "export {};\n".into(),
                    FileKind::Skipped,
                )
            }));
        }
    }
    files
}

/// The version range of an npm package in [`EXTERNALS`].
fn external_version(name: &str) -> &'static str {
    EXTERNALS.iter().find(|(external, ..)| *external == name).map_or("*", |(_, version, _)| version)
}

/// The relative specifier from a file in `from_dir` to the module `to`, both
/// relative to the same directory, without an extension.
fn relative_specifier(from_dir: &str, to: &str) -> String {
    let segments = |path: &'_ str| -> Vec<String> {
        path.split('/').filter(|segment| !segment.is_empty()).map(String::from).collect()
    };
    let (to_dir, name) = to.rsplit_once('/').unwrap_or(("", to));
    let (from, to_dir) = (segments(from_dir), segments(to_dir));
    // Only directories are shared: a module can be named like a directory
    // that it isn't in, as `components/table.tsx` is for `components/table/`.
    let common = from.iter().zip(&to_dir).take_while(|(a, b)| a == b).count();
    let mut rest = to_dir[common..].to_vec();
    rest.push(name.to_string());
    let rest = rest.join("/");
    match from.len() - common {
        0 => format!("./{rest}"),
        ups => format!("{}{rest}", "../".repeat(ups)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_specifiers() {
        assert_eq!(relative_specifier("src/a/b", "src/a/c/d"), "../c/d");
        assert_eq!(relative_specifier("src/a", "src/a/b"), "./b");
        assert_eq!(relative_specifier("src", "src/features/x/index"), "./features/x/index");
        assert_eq!(relative_specifier("a/c/table", "a/c/table"), "../table");
        assert_eq!(relative_specifier("a/c/table", "a/c/table/form"), "./form");
    }
}
