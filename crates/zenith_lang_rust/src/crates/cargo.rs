//! Finding the crates by asking Cargo, which knows its own targets and
//! dependencies better than their manifests say.

use std::{
    env, fmt, io,
    path::{Path, PathBuf},
    process::Command,
};

use log::{debug, warn};
use rustc_hash::FxHashMap;
use serde::Deserialize;
use zenith_diagnostics::message;
use zenith_utils::fs::adjust_canonicalisation;

use super::{Crates, Declared, Found, Target, TargetKind, edition};
use crate::manifest::Manifest;

/// Why Cargo could not say what the crates are.
#[derive(Debug)]
pub(super) enum CargoError {
    /// `cargo` could not be run, as it is not installed.
    NotFound,

    /// `cargo` ran, or could not start, but gave no metadata: it says why.
    Failed(String),
}

impl CargoError {
    /// Log the error, as the reason to read the manifests instead.
    pub(super) fn log(&self) {
        match self {
            Self::NotFound => debug!("{}", message!(rust_reading_manifests)),
            Self::Failed(error) => warn!("{}", message!(rust_cargo_failed, error)),
        }
    }
}

impl fmt::Display for CargoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str(&message!(rust_cargo_not_installed)),
            Self::Failed(error) => f.write_str(error),
        }
    }
}

impl Crates {
    /// The crates of the packages in `manifests`, each with its directory, as
    /// Cargo says they are when asked from the project at `root`.
    pub(super) fn from_cargo(
        root: &Path,
        manifests: &[(&Path, Manifest)],
    ) -> Result<Self, CargoError> {
        // A workspace is asked about once, and a package of one is among its members.
        let mut found: FxHashMap<PathBuf, Found> = FxHashMap::default();
        for dir in asked(manifests) {
            for package in metadata(root, &dir.join("Cargo.toml"))?.packages {
                if let Some(package) = package.found() {
                    found.insert(package.dir.clone(), package);
                }
            }
        }
        Ok(Self::new(found.into_values().collect()))
    }
}

/// The directories of the manifests to ask Cargo about: those of workspaces,
/// and of packages that are in none.
pub(super) fn asked<'m>(manifests: &'m [(&Path, Manifest)]) -> impl Iterator<Item = &'m Path> {
    manifests.iter().filter_map(move |(dir, manifest)| {
        let is_workspace = manifest.workspace.is_some();
        let is_covered = manifests.iter().any(|(above, other)| {
            other.workspace.is_some() && dir != above && dir.starts_with(above)
        });
        (is_workspace || manifest.package.is_some() && !is_covered).then_some(*dir)
    })
}

/// What Cargo says of the workspace that the manifest at `manifest` is in,
/// run from `root`.
fn metadata(root: &Path, manifest: &Path) -> Result<Metadata, CargoError> {
    // Without the dependencies, Cargo resolves no versions, reads no lockfile and
    // needs neither the network nor the registry: the packages of the workspace
    // are all it says.
    let output = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--no-deps", "--offline", "--manifest-path"])
        .arg(manifest)
        .current_dir(root)
        // A toolchain that the project pins and isn't installed is an error, not a download.
        .env("RUSTUP_AUTO_INSTALL", "0")
        .output()
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => CargoError::NotFound,
            _ => CargoError::Failed(error.to_string()),
        })?;
    if !output.status.success() {
        return Err(CargoError::Failed(String::from_utf8_lossy(&output.stderr).trim().to_owned()));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| CargoError::Failed(error.to_string()))
}

/// The output of `cargo metadata`, as far as zenith reads it.
#[derive(Debug, Deserialize)]
struct Metadata {
    /// The packages of the workspace.
    packages: Vec<MetadataPackage>,
}

/// A package in the metadata.
#[derive(Debug, Deserialize)]
struct MetadataPackage {
    /// The name of the package.
    name: String,

    /// The path of its `Cargo.toml`.
    manifest_path: PathBuf,

    /// The edition of Rust that its code is written in, such as `"2021"`.
    edition: Option<String>,

    /// Its targets.
    targets: Vec<MetadataTarget>,

    /// Its dependencies, of every kind.
    dependencies: Vec<MetadataDependency>,
}

/// A target in the metadata.
#[derive(Debug, Deserialize)]
struct MetadataTarget {
    /// The name of the target, which for a library is what code calls it.
    name: String,

    /// What the target is, such as `bin`, or `rlib` for a library.
    kind: Vec<String>,

    /// The path of the root module.
    src_path: PathBuf,
}

/// A dependency in the metadata.
#[derive(Debug, Deserialize)]
struct MetadataDependency {
    /// The name of the package that it is.
    name: String,

    /// The name that the code uses for it, if it isn't the package's.
    rename: Option<String>,

    /// Its directory, if it is a path.
    path: Option<PathBuf>,
}

impl MetadataPackage {
    /// The package, as found. Its paths are canonicalised as the project root
    /// is, since Cargo writes them as it was given them, or, for a path
    /// dependency, as it joined them; on Windows, it may leave out the `\\?\`
    /// prefix that the root has, or add it.
    fn found(self) -> Option<Found> {
        let dir = adjust_canonicalisation(self.manifest_path.parent()?);
        let targets: Vec<_> = (self.targets.iter())
            .map(|target| Target {
                kind: target.kind(),
                root: adjust_canonicalisation(&target.src_path),
            })
            .collect();
        let library = self.targets.iter().find(|target| target.kind() == TargetKind::Lib);
        let dependencies = (self.dependencies.into_iter())
            .map(|dependency| Declared {
                name: dependency.rename.as_deref().unwrap_or(&dependency.name).replace('-', "_"),
                package: dependency.name,
                path: dependency.path.map(adjust_canonicalisation),
            })
            .collect();
        Some(Found {
            name: self.name,
            dir,
            edition: edition(self.edition.as_deref()),
            library: library.map(|target| target.name.clone()),
            targets,
            dependencies,
        })
    }
}

impl MetadataTarget {
    /// What the target is: one of the library's many kinds, or a binary, a
    /// test, a benchmark, an example or the build script.
    fn kind(&self) -> TargetKind {
        match self.kind.first().map(String::as_str) {
            Some("bin") => TargetKind::Bin,
            Some("test") => TargetKind::Test,
            Some("bench") => TargetKind::Bench,
            Some("example") => TargetKind::Example,
            Some("custom-build") => TargetKind::Build,
            _ => TargetKind::Lib,
        }
    }
}
