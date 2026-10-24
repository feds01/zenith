//! A project on disk, for the tests of discovery and resolution.

use std::{
    fs,
    path::{Path, PathBuf},
};

use tempfile::TempDir;
use zenith_utils::{fs::disk, path::adjust_canonicalisation};

use crate::{modules::Roots, packages::Package};

/// A project in a temporary directory.
pub(crate) struct Project {
    /// The directory, which is removed with the project.
    _dir: TempDir,

    /// The canonical root of the project.
    root: PathBuf,

    /// The `pyproject.toml` files of the project.
    manifests: Vec<PathBuf>,
}

impl Project {
    /// A project with these `files`, each a path relative to its root and its
    /// contents. A `pyproject.toml` is a manifest of the project.
    pub(crate) fn new(files: &[(&str, &str)]) -> Self {
        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        let mut manifests = Vec::new();
        for (name, contents) in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            if path.file_name().is_some_and(|name| name == "pyproject.toml") {
                manifests.push(path);
            }
        }
        manifests.sort();
        Self { _dir: dir, root, manifests }
    }

    /// A workspace of two packages, whose root only configures tools:
    /// `acme-api`, in the src layout, whose `acme.api` package is in the
    /// `acme` namespace package, and `acme-core`, in the flat layout, with
    /// the regular package `core`. Beside them are a `scripts` directory of
    /// loose files, a `tests` directory whose files import each other, and an
    /// `email` namespace directory that shares its name with the standard
    /// library.
    pub(crate) fn workspace() -> Self {
        Self::new(&[
            ("pyproject.toml", include_str!("fixtures/workspace.toml")),
            ("services/api/pyproject.toml", include_str!("fixtures/api.toml")),
            ("services/api/src/acme/api/__init__.py", ""),
            ("services/api/src/acme/api/handlers.py", ""),
            ("services/api/src/acme/api/models/__init__.py", ""),
            ("services/api/src/acme/api/models/user.py", ""),
            ("services/api/src/acme/api/stubs.pyi", ""),
            ("services/api/tests/test_handlers.py", ""),
            ("services/core/pyproject.toml", include_str!("fixtures/core.toml")),
            ("services/core/core/__init__.py", ""),
            ("services/core/core/db.py", ""),
            ("services/core/core/logging.py", ""),
            ("scripts/build.py", ""),
            ("scripts/helpers.py", ""),
            ("tests/conftest.py", ""),
            ("tests/util.py", ""),
            ("email/templates.py", ""),
            ("legacy/app/__init__.py", ""),
            ("legacy/app/views.py", ""),
            ("legacy/app/models.py", ""),
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

    /// The packages of the project.
    pub(crate) fn packages(&self) -> Vec<Package> {
        Package::read_all(disk().as_ref(), &self.manifests)
    }

    /// The roots of the project, found from its packages.
    pub(crate) fn roots(&self) -> Roots {
        Roots::inferred(disk().as_ref(), &self.root, &self.packages())
    }
}
