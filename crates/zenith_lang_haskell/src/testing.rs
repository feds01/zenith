//! A project on disk, for the tests of resolution.

use std::{
    fs,
    path::{Path, PathBuf},
};

use tempfile::TempDir;
use zenith_utils::fs::{adjust_canonicalisation, disk};

use crate::packages::{Build, Installed, Package, Packages};

/// A project in a temporary directory.
pub(crate) struct Project {
    /// The directory, which is removed with the project.
    _dir: TempDir,

    /// The canonical root of the project.
    root: PathBuf,

    /// The `.cabal` descriptions of its packages.
    descriptions: Vec<PathBuf>,
}

impl Project {
    /// A project with these `files`, each a path relative to its root and its
    /// contents.
    pub(crate) fn new(files: &[(&str, &str)]) -> Self {
        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        let mut descriptions = Vec::new();
        for (name, contents) in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            if path.extension().is_some_and(|extension| extension == "cabal") {
                descriptions.push(path);
            }
        }
        Self { _dir: dir, root, descriptions }
    }

    /// The acme project: `acme-core`, a library, and `acme-api`, a library,
    /// an executable and a test suite, as a real build with cabal described
    /// them, and `acme-prelude`, whose executable sees `relude` for its
    /// Prelude, and whose library depends on two packages that expose
    /// `Data.Map`, and re-exports one's. Beside them is a script that no
    /// package compiles.
    pub(crate) fn acme() -> Self {
        Self::new(&[
            ("cabal.project", "packages: core api prelude\n"),
            ("core/acme-core.cabal", include_str!("packages/fixtures/acme-core.cabal")),
            ("core/src/Acme/Core.hs", ""),
            ("core/src/Acme/Core/Types.hs", ""),
            ("core/src/Acme/Core/Types.hs-boot", ""),
            ("api/acme-api.cabal", include_str!("packages/fixtures/acme-api.cabal")),
            ("api/src/Acme/Api.hs", ""),
            ("api/app/Main.hs", ""),
            ("api/test/Spec.hs", ""),
            ("prelude/acme-prelude.cabal", include_str!("packages/fixtures/acme-prelude.cabal")),
            ("prelude/src/Acme/Prelude.hs", ""),
            ("prelude/internal/Acme/Internal.hs", ""),
            ("prelude/tool/Tool.hs", ""),
            ("scripts/Migrate.hs", ""),
        ])
    }

    /// The canonical root of the project.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The absolute path of the file `name`, relative to the root.
    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Its packages, as built: their dependencies are `base`, `containers`
    /// and `split`, as a real build installed them, and `relude` and
    /// `legacy-containers`.
    pub(crate) fn packages(&self) -> Packages {
        let installed = installed(&[
            include_str!("packages/fixtures/base.conf"),
            include_str!("packages/fixtures/containers.conf"),
            include_str!("packages/fixtures/split.conf"),
            include_str!("packages/fixtures/relude.conf"),
            include_str!("packages/fixtures/legacy-containers.conf"),
        ]);
        Packages::new(
            self.local(),
            vec![Build::new(Some(self.root.clone()), installed)],
            Build::default(),
        )
    }

    /// Its packages, never built, with only the compiler's `base` and
    /// `containers` to see.
    pub(crate) fn unbuilt(&self) -> Packages {
        let compiler = installed(&[
            include_str!("packages/fixtures/base.conf"),
            include_str!("packages/fixtures/containers.conf"),
        ]);
        Packages::new(self.local(), Vec::new(), Build::new(None, compiler))
    }

    /// The packages that its descriptions describe.
    fn local(&self) -> Vec<Package> {
        self.descriptions.iter().filter_map(|path| Package::read(&**disk(), path)).collect()
    }
}

/// The libraries of the database entries `entries`.
fn installed(entries: &[&str]) -> Vec<Installed> {
    entries.iter().filter_map(|entry| Installed::parse(entry)).collect()
}
