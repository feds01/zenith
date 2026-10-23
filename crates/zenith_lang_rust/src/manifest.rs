//! `Cargo.toml`, as far as zenith reads it: the package, its targets, its
//! dependencies, and the dependencies that a workspace shares.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use log::warn;
use serde::{Deserialize, de::IgnoredAny};
use zenith_utils::{fs::read_text, path::normalize_path};

use crate::crates::{Declared, Target, TargetKind};

/// A `Cargo.toml`. Everything else in it is left unread.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Manifest {
    /// The `[package]` table. A manifest without one only declares a
    /// workspace.
    pub(crate) package: Option<Package>,

    /// The `[lib]` table.
    lib: Option<Lib>,

    /// The `[[bin]]` tables.
    bin: Vec<TargetTable>,

    /// The `[[test]]` tables.
    test: Vec<TargetTable>,

    /// The `[[bench]]` tables.
    bench: Vec<TargetTable>,

    /// The `[[example]]` tables.
    example: Vec<TargetTable>,

    /// The `[workspace]` table.
    pub(crate) workspace: Option<Workspace>,

    /// The `[dependencies]`, `[dev-dependencies]` and `[build-dependencies]`
    /// tables.
    #[serde(flatten)]
    dependencies: DependencyTables,

    /// The same tables for each target, as `[target.'cfg(unix)'.dependencies]`.
    target: BTreeMap<String, DependencyTables>,
}

/// The `[package]` table.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Package {
    /// The name of the package.
    pub(crate) name: String,

    /// The edition of Rust that its code is written in.
    edition: Option<Inheritable>,

    /// The build script: a path, or `false` for none.
    build: Option<Build>,
}

/// A key of a package that may take its value from the workspace's, with
/// `{ workspace = true }`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Inheritable {
    /// The value itself.
    Value(String),

    /// The workspace's value, if `workspace` is `true`.
    Workspace {
        /// Whether to take the workspace's value.
        workspace: bool,
    },
}

/// The `build` key of a package.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Build {
    /// `build = false` has none, and `build = true` is the default.
    Enabled(bool),

    /// The path of the script, relative to the package.
    Path(String),
}

/// The `[lib]` table.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Lib {
    /// The name of the library, which the code that uses it writes. It
    /// defaults to the name of the package, with `_` for `-`.
    name: Option<String>,

    /// The root of the library, relative to the package.
    path: Option<String>,
}

/// A `[[bin]]`, `[[test]]`, `[[bench]]` or `[[example]]` table, whose path
/// may be outside the directory that Cargo looks in by itself.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TargetTable {
    /// The root of the target, relative to the package.
    path: Option<String>,
}

/// The `[workspace]` table.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Workspace {
    /// The dependencies that its members share, by `workspace = true`.
    dependencies: BTreeMap<String, DependencySpec>,

    /// The keys of `[package]` that its members share, by `workspace = true`.
    package: WorkspacePackage,
}

/// The `[workspace.package]` table.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct WorkspacePackage {
    /// The edition of Rust that the members' code is written in.
    edition: Option<String>,
}

/// The tables that declare dependencies, at the top of a manifest or for a
/// target.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DependencyTables {
    /// The dependencies of the package.
    dependencies: BTreeMap<String, DependencySpec>,

    /// The dependencies of its tests, examples and benchmarks.
    #[serde(rename = "dev-dependencies")]
    dev_dependencies: BTreeMap<String, DependencySpec>,

    /// The dependencies of its build script.
    #[serde(rename = "build-dependencies")]
    build_dependencies: BTreeMap<String, DependencySpec>,
}

/// A dependency, as the table that declares it writes it.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DependencySpec {
    /// A table: `serde = { version = "1", features = ["derive"] }`.
    Table(DependencyTable),

    /// Only a version requirement: `serde = "1"`. It is tried last, as it
    /// accepts anything.
    Version(IgnoredAny),
}

/// The keys of a dependency's table that decide what it is.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DependencyTable {
    /// The directory of a dependency that is a path, relative to the manifest.
    path: Option<String>,

    /// The package that the dependency is, when it is renamed.
    package: Option<String>,

    /// Whether the dependency is declared in the `[workspace.dependencies]`
    /// of the workspace.
    workspace: bool,
}

impl Manifest {
    /// The manifests at `paths` that can be read, each with its directory.
    /// Those that can't are warned about, and left out.
    pub(crate) fn read_all(paths: &[PathBuf]) -> Vec<(&Path, Manifest)> {
        paths
            .iter()
            .filter_map(|path| {
                let manifest = read_text(path)
                    .map_err(|error| error.to_string())
                    .and_then(|text| toml::from_str(&text).map_err(|error| error.to_string()));
                match manifest {
                    Ok(manifest) => Some((path.parent()?, manifest)),
                    Err(error) => {
                        warn!("could not read `{}`: {error}", path.display());
                        None
                    }
                }
            })
            .collect()
    }

    /// The edition of the package, such as `"2021"`, if it names one, from
    /// `workspace`, the manifest of its workspace, if it takes the
    /// workspace's.
    pub(crate) fn edition<'m>(&'m self, workspace: Option<&'m Manifest>) -> Option<&'m str> {
        match self.package.as_ref()?.edition.as_ref()? {
            Inheritable::Value(edition) => Some(edition),
            Inheritable::Workspace { workspace: false } => None,
            Inheritable::Workspace { workspace: true } => {
                workspace?.workspace.as_ref()?.package.edition.as_deref()
            }
        }
    }

    /// The name that the code uses for the library of the package, which is
    /// the name of the package with `_` for `-` unless the manifest says
    /// otherwise.
    pub(crate) fn library_name(&self) -> String {
        let name = self.lib.as_ref().and_then(|lib| lib.name.clone());
        name.unwrap_or_else(|| {
            self.package.as_ref().map_or_else(String::new, |package| package.name.replace('-', "_"))
        })
    }

    /// The targets of the package in `dir`: those that Cargo finds by itself,
    /// in the places that it looks, and those that the manifest gives a path.
    pub(crate) fn targets(&self, dir: &Path) -> Vec<Target> {
        let mut targets = Vec::new();
        let mut add = |kind: TargetKind, root: PathBuf| targets.push(Target { kind, root });

        // 1. The library, which is in `src/lib.rs` unless the manifest says otherwise.
        let default = dir.join("src/lib.rs");
        let lib = match &self.lib {
            Some(lib) => Some(lib.path.as_ref().map_or(default, |path| dir.join(path))),
            None => default.is_file().then_some(default),
        };
        if let Some(root) = lib {
            add(TargetKind::Lib, root);
        }

        // 2. The binaries, tests, benchmarks and examples.
        let main = dir.join("src/main.rs");
        if main.is_file() {
            add(TargetKind::Bin, main);
        }
        let kinds = [
            (TargetKind::Bin, "src/bin", &self.bin),
            (TargetKind::Test, "tests", &self.test),
            (TargetKind::Bench, "benches", &self.bench),
            (TargetKind::Example, "examples", &self.example),
        ];
        for (kind, subdir, tables) in kinds {
            for root in discover_roots(&dir.join(subdir)) {
                add(kind, root);
            }
            for path in tables.iter().filter_map(|table| table.path.as_ref()) {
                add(kind, dir.join(path));
            }
        }

        // 3. The build script.
        let build = self.package.as_ref().and_then(|package| package.build.as_ref());
        match build {
            Some(Build::Enabled(false)) => {}
            Some(Build::Path(path)) => add(TargetKind::Build, dir.join(path)),
            _ => {
                let script = dir.join("build.rs");
                if script.is_file() {
                    add(TargetKind::Build, script);
                }
            }
        }
        targets
    }

    /// The dependencies of the package in `dir`, as its code names them.
    /// `workspace` is the directory and manifest of the workspace that the
    /// package is in, which declares the dependencies that `workspace = true`
    /// takes.
    pub(crate) fn declared(
        &self,
        dir: &Path,
        workspace: Option<(&Path, &Manifest)>,
    ) -> Vec<Declared> {
        let tables = std::iter::once(&self.dependencies).chain(self.target.values());
        let all = tables.flat_map(|tables| {
            let DependencyTables { dependencies, dev_dependencies, build_dependencies } = tables;
            dependencies.iter().chain(dev_dependencies).chain(build_dependencies)
        });
        all.map(|(key, spec)| {
            // 1. A dependency of the workspace is declared in its manifest, where paths are
            //    relative to it.
            let shared = workspace.and_then(|(workspace_dir, manifest)| {
                let spec = manifest.workspace.as_ref()?.dependencies.get(key)?;
                Some((workspace_dir, spec))
            });
            let (base, spec) = match spec {
                DependencySpec::Table(table) if table.workspace => shared.unwrap_or((dir, spec)),
                _ => (dir, spec),
            };

            // 2. A dependency may be a path, and may be renamed.
            let (path, package) = match spec {
                DependencySpec::Version(_) => (None, None),
                DependencySpec::Table(table) => (table.path.as_deref(), table.package.as_deref()),
            };
            Declared {
                name: key.replace('-', "_"),
                package: package.unwrap_or(key).to_owned(),
                path: path.map(|path| normalize_path(base.join(path))),
            }
        })
        .collect()
    }
}

/// The roots of the targets in `dir`, one of the directories in which Cargo
/// looks for targets: each `.rs` file in it, and the `main.rs` of each of its
/// directories.
fn discover_roots(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let root = match path.is_dir() {
                true => path.join("main.rs"),
                false => path,
            };
            (root.extension().is_some_and(|extension| extension == "rs") && root.is_file())
                .then_some(root)
        })
        .collect()
}
