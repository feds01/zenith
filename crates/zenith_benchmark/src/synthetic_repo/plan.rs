//! Planning a synthetic repository: its packages, their features, and the
//! path, kind and exported symbol of every source file, before any contents
//! are generated.

use std::collections::{BTreeMap, BTreeSet};

use fastrand::Rng;
use heck::{ToLowerCamelCase, ToUpperCamelCase};

use super::{PackageRole, SyntheticPackage};

/// About how many source files a package has, which sets how many packages a
/// repository has.
const FILES_PER_PACKAGE: usize = 250;

/// The scope of every workspace package.
const SCOPE: &str = "@acme";

/// Library names, given out in order and suffixed with `-2`, `-3`, … once
/// they run out. So are the other name lists.
const LIBRARY_NAMES: &[&str] = &[
    "core",
    "ui",
    "utils",
    "config",
    "auth",
    "db",
    "design-system",
    "analytics",
    "billing",
    "search",
    "notifications",
    "feature-flags",
    "logger",
    "i18n",
    "forms",
    "charts",
    "editor",
    "storage",
    "email",
    "payments",
    "telemetry",
    "testing",
    "icons",
    "api-client",
    "schemas",
    "permissions",
    "workflows",
    "queue",
    "cache",
    "media",
];

/// Service names.
const SERVICE_NAMES: &[&str] =
    &["api", "worker", "scheduler", "webhooks", "gateway", "ingest", "exporter", "mailer"];

/// App names.
const APP_NAMES: &[&str] =
    &["web", "admin", "docs", "dashboard", "mobile", "marketing", "studio", "portal"];

/// Feature names. Each package starts at a different point in the list.
const FEATURE_NAMES: &[&str] = &[
    "accounts",
    "billing",
    "invoices",
    "users",
    "teams",
    "projects",
    "search",
    "reports",
    "settings",
    "onboarding",
    "notifications",
    "dashboard",
    "calendar",
    "files",
    "comments",
    "audit-log",
    "integrations",
    "checkout",
    "catalog",
    "orders",
    "inventory",
    "messaging",
    "profile",
    "sessions",
    "exports",
    "alerts",
    "metrics",
    "tags",
    "roles",
    "activity",
];

/// Component file names.
const COMPONENT_NAMES: &[&str] = &[
    "list",
    "table",
    "card",
    "form",
    "dialog",
    "header",
    "panel",
    "row",
    "summary",
    "filters",
    "editor",
    "picker",
    "badge",
    "menu",
    "toolbar",
    "chart",
    "empty-state",
    "skeleton",
    "details",
    "avatar",
];

/// Subdirectories of `components/` that some components are nested in.
const COMPONENT_GROUPS: &[&str] = &["forms", "table", "layout"];

/// Hook file names, after `use-`.
const HOOK_NAMES: &[&str] = &[
    "query",
    "mutation",
    "filters",
    "pagination",
    "selection",
    "permissions",
    "debounce",
    "local-state",
    "subscription",
    "toast",
];

/// API client file names.
const CLIENT_NAMES: &[&str] =
    &["client", "queries", "mutations", "schema", "endpoints", "transport", "cache", "errors"];

/// Utility file names.
const UTIL_NAMES: &[&str] = &[
    "format",
    "parse",
    "sort",
    "group",
    "validate",
    "normalise",
    "merge",
    "diff",
    "serialise",
    "constants",
];

/// Service handler file names.
const HANDLER_NAMES: &[&str] =
    &["create", "update", "delete", "list", "get", "search", "import", "export"];

/// Service job file names.
const JOB_NAMES: &[&str] = &["sync", "cleanup", "reindex", "digest", "retry", "backfill"];

/// Store file names.
const STORE_NAMES: &[&str] = &["table", "queries", "migrations", "repository", "seed"];

impl PackageRole {
    /// How big a package is relative to the others: apps tend to be the
    /// biggest, and libraries the smallest.
    fn weight(self) -> usize {
        match self {
            Self::Library => 1,
            Self::Service => 2,
            Self::App => 3,
        }
    }

    /// Choose the libraries that the `nth` package of this role depends on.
    /// The libraries are the first `libraries` plans.
    fn choose_dependencies(self, rng: &mut Rng, nth: usize, libraries: usize) -> Vec<usize> {
        let (candidates, wanted) = match self {
            Self::Library => (nth, rng.usize(0..=3)),
            Self::Service => (libraries, rng.usize(2..=4)),
            Self::App => (libraries, rng.usize(3..=6)),
        };

        let mut chosen: Vec<usize> = (0..candidates).collect();
        rng.shuffle(&mut chosen);
        chosen.truncate(wanted);
        chosen.sort_unstable();
        chosen
    }

    /// Pick the category of a feature module, weighted by the role.
    fn pick_category(self, rng: &mut Rng) -> Category {
        let weights: &[(Category, u32)] = match self {
            Self::Library => &[
                (Category::Component, 25),
                (Category::Hook, 15),
                (Category::Client, 15),
                (Category::Util, 25),
                (Category::Store, 5),
                (Category::Test, 15),
            ],
            Self::Service => &[
                (Category::Handler, 30),
                (Category::Job, 15),
                (Category::Store, 20),
                (Category::Util, 20),
                (Category::Test, 15),
            ],
            Self::App => &[
                (Category::Component, 40),
                (Category::Hook, 20),
                (Category::Client, 15),
                (Category::Util, 10),
                (Category::Test, 15),
            ],
        };

        let total: u32 = weights.iter().map(|(_, weight)| weight).sum();
        let mut roll = rng.u32(..total);
        for &(category, weight) in weights {
            if roll < weight {
                return category;
            }
            roll -= weight;
        }
        Category::Util
    }
}

/// The kind of module that a source file is, which decides its path, its
/// imports and its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Category {
    /// A library's `src/index.ts`, re-exporting its features.
    PackageBarrel,

    /// A library's `src/server.ts`, exported as `./server`.
    Server,

    /// An app's `src/main.tsx`.
    AppEntry,

    /// A service's `src/main.ts`.
    ServiceEntry,

    /// A service's CommonJS `scripts/seed.cjs`.
    Script,

    /// A feature's `index.ts`, re-exporting its modules.
    FeatureBarrel,

    /// A feature's `types.ts`.
    Types,

    /// An app route, `page.tsx`.
    Route,

    /// A React component.
    Component,

    /// A React hook.
    Hook,

    /// An API client.
    Client,

    /// A utility module.
    Util,

    /// A service's HTTP handler.
    Handler,

    /// A service's background job.
    Job,

    /// A database table and its queries.
    Store,

    /// A test of another module in its feature.
    Test,
}

impl Category {
    /// Whether other modules import this one by its exported symbol.
    pub(super) fn is_importable(self) -> bool {
        matches!(
            self,
            Self::Component
                | Self::Hook
                | Self::Client
                | Self::Util
                | Self::Handler
                | Self::Job
                | Self::Store
        )
    }

    /// The extension of a module of this category, without the `.`.
    fn extension(self) -> &'static str {
        match self {
            Self::AppEntry | Self::Route | Self::Component => "tsx",
            Self::Script => "cjs",
            _ => "ts",
        }
    }
}

/// A source file of a package, before its contents are generated.
pub(super) struct Module {
    /// Its path relative to the package directory, without the extension,
    /// e.g. `src/features/billing/hooks/use-query`.
    pub(super) stem: String,

    /// What kind of module it is.
    pub(super) category: Category,

    /// The feature it belongs to, as an index into the package's features.
    pub(super) feature: Option<usize>,

    /// The name that it exports, which importers import.
    pub(super) symbol: String,

    /// For a test, the module that it tests.
    pub(super) subject: Option<usize>,
}

/// A feature directory of a package.
pub(super) struct Feature {
    /// Its name, e.g. `billing` or `audit-log-2`.
    pub(super) name: String,

    /// Its directory relative to the package, e.g. `src/features/billing`,
    /// or `src/features/billing/invoices` when it's nested in another.
    pub(super) dir: String,

    /// The feature it's nested in.
    pub(super) parent: Option<usize>,

    /// Whether the package barrel re-exports it as a namespace.
    pub(super) namespaced: bool,

    /// Its modules, as indices into the package's modules, barrel first.
    pub(super) modules: Vec<usize>,
}

/// A package, before its files are generated.
pub(super) struct PackagePlan {
    /// The package as reported.
    pub(super) package: SyntheticPackage,

    /// The libraries it depends on, as indices into the plans.
    pub(super) dependencies: Vec<usize>,

    /// Its source files.
    pub(super) modules: Vec<Module>,

    /// Its feature directories.
    pub(super) features: Vec<Feature>,
}

impl PackagePlan {
    /// The importable modules of a feature.
    pub(super) fn importables(&self, feature: usize) -> Vec<usize> {
        self.features[feature]
            .modules
            .iter()
            .copied()
            .filter(|&module| self.modules[module].category.is_importable())
            .collect()
    }

    /// The path of one of the package's modules, relative to the root.
    pub(super) fn path(&self, module: &Module) -> String {
        format!("{}/{}.{}", self.package.dir, module.stem, module.category.extension())
    }

    /// The package name without its scope, e.g. `design-system`.
    pub(super) fn short_name(&self) -> &str {
        let name = &self.package.name;
        name.strip_prefix(SCOPE).and_then(|name| name.strip_prefix('/')).unwrap_or(name)
    }

    /// Plan exactly `quota` source files: the package's entry points, then
    /// features of 8 to 24 files until the quota is used up. `offset` is where
    /// the feature names start.
    fn plan_modules(&mut self, rng: &mut Rng, quota: usize, offset: usize) {
        let fixed: &[(Category, &str)] = match self.package.role {
            PackageRole::Library if self.package.has_exports => {
                &[(Category::PackageBarrel, "src/index"), (Category::Server, "src/server")]
            }
            PackageRole::Library => &[(Category::PackageBarrel, "src/index")],
            PackageRole::Service => {
                &[(Category::ServiceEntry, "src/main"), (Category::Script, "scripts/seed")]
            }
            PackageRole::App => &[(Category::AppEntry, "src/main")],
        };

        let mut stems = BTreeSet::new();
        for &(category, stem) in fixed {
            let symbol = match category {
                Category::Server => {
                    format!("load{}Config", self.short_name().to_upper_camel_case())
                }
                _ => String::new(),
            };
            let module =
                Module { stem: stem.to_string(), category, feature: None, symbol, subject: None };
            self.add_module(&mut stems, module);
        }

        let mut remaining = quota - self.modules.len();
        while remaining > 0 {
            let size = rng.usize(8..=24).min(remaining);
            self.plan_feature(rng, &mut stems, size, offset);
            remaining -= size;
        }
    }

    /// Plan a feature of `size` source files: a barrel, its types, an app's
    /// routes, and modules of the package role's categories.
    fn plan_feature(
        &mut self,
        rng: &mut Rng,
        stems: &mut BTreeSet<String>,
        size: usize,
        offset: usize,
    ) {
        // 1. Add the feature. Every third one is nested in the one before it, which
        //    never is.
        let feature = self.features.len();
        let name = cycled_from(FEATURE_NAMES, feature, offset);
        let parent = (feature % 3 == 2).then(|| feature - 1);
        let dir = match parent {
            Some(parent) => format!("{}/{name}", self.features[parent].dir),
            None => format!("src/features/{name}"),
        };
        let namespaced = parent.is_none() && feature % 4 == 3;
        self.features.push(Feature {
            name: name.clone(),
            dir: dir.clone(),
            parent,
            namespaced,
            modules: Vec::new(),
        });

        // 2. Add its barrel, its types and, in an app, its routes.
        let module = |stem: String, category: Category, symbol: String| Module {
            stem,
            category,
            feature: Some(feature),
            symbol,
            subject: None,
        };
        let title = name.to_upper_camel_case();
        self.add_module(
            stems,
            module(format!("{dir}/index"), Category::FeatureBarrel, String::new()),
        );
        let mut left = size - 1;
        if left > 0 {
            let types = module(format!("{dir}/types"), Category::Types, format!("{title}Record"));
            self.add_module(stems, types);
            left -= 1;
        }
        if self.package.role == PackageRole::App && parent.is_none() && left >= 4 {
            let route = format!("src/app/(dashboard)/{name}");
            let page = module(format!("{route}/page"), Category::Route, format!("{title}Page"));
            self.add_module(stems, page);
            let detail =
                module(format!("{route}/[id]/page"), Category::Route, format!("{title}Detail"));
            self.add_module(stems, detail);
            left -= 2;
        }

        // 3. Fill the rest with modules of the role's categories, each named after the
        //    next name of its category's list.
        let mut counters: BTreeMap<Category, usize> = BTreeMap::new();
        for _ in 0..left {
            let importables = self.importables(feature);
            let mut category = self.package.role.pick_category(rng);
            if category == Category::Test && importables.is_empty() {
                category = Category::Util;
            }
            let counter = counters.entry(category).or_insert(0);
            let nth = *counter;
            *counter += 1;

            let mut subject = None;
            let (stem, symbol) = match category {
                Category::Component => {
                    let group = (rng.f32() < 0.3).then(|| rng.choice(COMPONENT_GROUPS)).flatten();
                    let base = cycled(COMPONENT_NAMES, nth);
                    let nested = group.map(|group| format!("{group}/")).unwrap_or_default();
                    let stem =
                        unique_stem(stems, |n| format!("{dir}/components/{nested}{base}{n}"));
                    let group = group.map(|group| group.to_upper_camel_case()).unwrap_or_default();
                    let symbol =
                        format!("{title}{group}{}", file_name(&stem).to_upper_camel_case());
                    (stem, symbol)
                }
                Category::Hook => {
                    let base = cycled(HOOK_NAMES, nth);
                    let stem = unique_stem(stems, |n| format!("{dir}/hooks/use-{base}{n}"));
                    let symbol =
                        format!("use{title}{}", file_name(&stem)[4..].to_upper_camel_case());
                    (stem, symbol)
                }
                Category::Client => {
                    let base = cycled(CLIENT_NAMES, nth);
                    let stem = unique_stem(stems, |n| format!("{dir}/api/{base}{n}"));
                    let symbol = format!("fetch{title}{}", file_name(&stem).to_upper_camel_case());
                    (stem, symbol)
                }
                Category::Util => {
                    let base = cycled(UTIL_NAMES, nth);
                    let stem = unique_stem(stems, |n| format!("{dir}/utils/{base}{n}"));
                    let symbol = format!("{}{title}", file_name(&stem).to_lower_camel_case());
                    (stem, symbol)
                }
                Category::Handler => {
                    let base = cycled(HANDLER_NAMES, nth);
                    let stem = unique_stem(stems, |n| format!("{dir}/handlers/{base}{n}"));
                    let symbol = format!("handle{title}{}", file_name(&stem).to_upper_camel_case());
                    (stem, symbol)
                }
                Category::Job => {
                    let base = cycled(JOB_NAMES, nth);
                    let stem = unique_stem(stems, |n| format!("{dir}/jobs/{base}{n}"));
                    let symbol = format!("run{title}{}Job", file_name(&stem).to_upper_camel_case());
                    (stem, symbol)
                }
                Category::Store => {
                    let base = cycled(STORE_NAMES, nth);
                    let stem = unique_stem(stems, |n| format!("{dir}/db/{base}{n}"));
                    let symbol = format!(
                        "{}{}Table",
                        name.to_lower_camel_case(),
                        file_name(&stem).to_upper_camel_case()
                    );
                    (stem, symbol)
                }
                _ => {
                    let tested = *rng.choice(&importables).expect("tests have a subject");
                    subject = Some(tested);
                    let base = file_name(&self.modules[tested].stem).to_string();
                    let stem = unique_stem(stems, |n| format!("{dir}/__tests__/{base}{n}.test"));
                    (stem, String::new())
                }
            };
            self.add_module(stems, Module { subject, ..module(stem, category, symbol) });
        }
    }

    /// Add a module, to its feature if it has one, and take its stem.
    fn add_module(&mut self, stems: &mut BTreeSet<String>, module: Module) {
        stems.insert(module.stem.clone());
        if let Some(feature) = module.feature {
            self.features[feature].modules.push(self.modules.len());
        }
        self.modules.push(module);
    }
}

/// Plan the packages and split the source files between them.
pub(super) fn plan_packages(rng: &mut Rng, source_files: usize) -> Vec<PackagePlan> {
    // 1. Give each package a role.
    let count = (source_files / FILES_PER_PACKAGE).max(3);
    let apps = (count / 8).max(1);
    let services = (count / 8).max(1);
    let libraries = count - apps - services;

    let mut roles = vec![PackageRole::Library; libraries];
    roles.extend(vec![PackageRole::Service; services]);
    roles.extend(vec![PackageRole::App; apps]);

    // 2. Split the files by weight, and hand the remainder out one at a time.
    let total: usize = roles.iter().map(|role| role.weight()).sum();
    let mut quotas: Vec<usize> =
        roles.iter().map(|role| source_files * role.weight() / total).collect();
    let mut remainder = source_files - quotas.iter().sum::<usize>();
    for quota in &mut quotas {
        if remainder == 0 {
            break;
        }
        *quota += 1;
        remainder -= 1;
    }

    // 3. Name each package, choose its dependencies and plan its modules.
    let mut plans: Vec<PackagePlan> = Vec::with_capacity(count);
    let mut nth_of_role = BTreeMap::new();
    for (index, (&role, &quota)) in roles.iter().zip(&quotas).enumerate() {
        let nth = nth_of_role.entry(role).or_insert(0);
        let (name, dir) = match role {
            PackageRole::Library => {
                let name = cycled(LIBRARY_NAMES, *nth);
                let dir = format!("packages/{}", name.to_lower_camel_case());
                (name, dir)
            }
            PackageRole::Service => {
                let name = cycled(SERVICE_NAMES, *nth);
                let dir = format!("services/{name}");
                (name, dir)
            }
            PackageRole::App => {
                let name = cycled(APP_NAMES, *nth);
                let dir = format!("apps/{name}");
                (name, dir)
            }
        };
        let has_exports = role == PackageRole::Library && *nth % 2 == 1;
        let dependencies = role.choose_dependencies(rng, *nth, libraries);
        *nth += 1;

        let mut plan = PackagePlan {
            package: SyntheticPackage {
                name: format!("{SCOPE}/{name}"),
                dir,
                role,
                has_exports,
                dependencies: Vec::new(),
            },
            dependencies,
            modules: Vec::new(),
            features: Vec::new(),
        };
        plan.plan_modules(rng, quota, index);
        plans.push(plan);
    }

    // 4. Name the packages that each one depends on.
    let names: Vec<String> = plans.iter().map(|plan| plan.package.name.clone()).collect();
    for plan in &mut plans {
        let mut dependencies: Vec<String> =
            plan.dependencies.iter().map(|&dependency| names[dependency].clone()).collect();
        dependencies.sort();
        plan.package.dependencies = dependencies;
    }
    plans
}

/// The last segment of a POSIX path.
pub(super) fn file_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

/// Make a stem that no other module of the package has, suffixing `-2`, `-3`,
/// … as needed. `make` builds the stem from the suffix.
fn unique_stem(stems: &BTreeSet<String>, make: impl Fn(&str) -> String) -> String {
    let mut stem = make("");
    let mut n = 2;
    while stems.contains(&stem) {
        stem = make(&format!("-{n}"));
        n += 1;
    }
    stem
}

/// The `index`th name of a list, suffixed with `-2`, `-3`, … each time the
/// list runs out.
fn cycled(names: &[&str], index: usize) -> String {
    cycled_from(names, index, 0)
}

/// Like [`cycled`], but starting `offset` names into the list.
fn cycled_from(names: &[&str], index: usize, offset: usize) -> String {
    let name = names[(index + offset) % names.len()];
    match index / names.len() {
        0 => name.to_string(),
        round => format!("{name}-{}", round + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_suffixed_when_their_list_runs_out() {
        assert_eq!(cycled(&["a", "b"], 1), "b");
        assert_eq!(cycled(&["a", "b"], 3), "b-2");
        assert_eq!(cycled_from(&["a", "b"], 3, 1), "a-2");
    }
}
