//! `Cargo.toml`, as far as zenith reads it: the package, its library, its
//! dependencies, and the dependencies that a workspace shares.

use std::collections::BTreeMap;

use serde::{Deserialize, de::IgnoredAny};

/// A `Cargo.toml`. Everything else in it is left unread.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Manifest {
    /// The `[package]` table. A manifest without one only declares a
    /// workspace.
    pub(crate) package: Option<Package>,

    /// The `[lib]` table.
    pub(crate) lib: Option<Lib>,

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
}

/// The `[lib]` table.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Lib {
    /// The name of the library, which the code that uses it writes. It
    /// defaults to the name of the package, with `_` for `-`.
    pub(crate) name: Option<String>,

    /// The root of the library, relative to the package. It defaults to
    /// `src/lib.rs`.
    pub(crate) path: Option<String>,
}

/// The `[workspace]` table.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Workspace {
    /// The dependencies that its members share, by `workspace = true`.
    pub(crate) dependencies: BTreeMap<String, Dependency>,
}

/// The tables that declare dependencies, at the top of a manifest or for a
/// target.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DependencyTables {
    /// The dependencies of the package.
    dependencies: BTreeMap<String, Dependency>,

    /// The dependencies of its tests, examples and benchmarks.
    #[serde(rename = "dev-dependencies")]
    dev_dependencies: BTreeMap<String, Dependency>,

    /// The dependencies of its build script.
    #[serde(rename = "build-dependencies")]
    build_dependencies: BTreeMap<String, Dependency>,
}

/// A dependency, as the table that declares it writes it.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Dependency {
    /// A table: `serde = { version = "1", features = ["derive"] }`.
    Table(DependencyTable),

    /// Only a version requirement: `serde = "1"`. It is tried last, as it
    /// accepts anything.
    Version(IgnoredAny),
}

/// The keys of a dependency's table that decide what it is.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct DependencyTable {
    /// The directory of a dependency that is a path, relative to the manifest.
    pub(crate) path: Option<String>,

    /// The package that the dependency is, when it is renamed.
    pub(crate) package: Option<String>,

    /// Whether the dependency is declared in the `[workspace.dependencies]`
    /// of the workspace.
    pub(crate) workspace: bool,
}

impl Manifest {
    /// Parse the text of a manifest.
    pub(crate) fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// The dependencies of every table, by the name that the manifest gives
    /// each, which is not the name that the code uses when it has a `-`.
    pub(crate) fn dependencies(&self) -> impl Iterator<Item = (&str, &Dependency)> {
        let tables = std::iter::once(&self.dependencies).chain(self.target.values());
        tables
            .flat_map(|tables| {
                let DependencyTables { dependencies, dev_dependencies, build_dependencies } =
                    tables;
                dependencies.iter().chain(dev_dependencies).chain(build_dependencies)
            })
            .map(|(name, dependency)| (name.as_str(), dependency))
    }
}
