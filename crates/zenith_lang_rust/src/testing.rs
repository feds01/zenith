//! A project on disk, for the tests of discovery and resolution.

use std::{
    fs,
    path::{Path, PathBuf},
};

use tempfile::TempDir;

use crate::crates::{Crates, Discovery};

/// A project in a temporary directory.
pub(crate) struct Project {
    /// The directory, which is removed with the project.
    _dir: TempDir,

    /// The canonical root of the project.
    root: PathBuf,

    /// The `Cargo.toml` files of the project.
    manifests: Vec<PathBuf>,
}

impl Project {
    /// A project with these `files`, each a path relative to its root and its
    /// contents. A `Cargo.toml` is a manifest of the project.
    pub(crate) fn new(files: &[(&str, &str)]) -> Self {
        let dir = TempDir::new().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let mut manifests = Vec::new();
        for (name, contents) in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            if path.file_name().is_some_and(|name| name == "Cargo.toml") {
                manifests.push(path);
            }
        }
        manifests.sort();
        Self { _dir: dir, root, manifests }
    }

    /// A workspace of three crates, whose root declares no package: `app`,
    /// which uses `core-lib` and `util-kit` and has a file for each kind of
    /// root, `core-lib`, and `util-kit`, whose library is `kit` in
    /// `lib/lib.rs`.
    pub(crate) fn workspace() -> Self {
        Self::new(&[
            ("Cargo.toml", include_str!("fixtures/workspace.toml")),
            ("crates/app/Cargo.toml", include_str!("fixtures/app.toml")),
            ("crates/app/build.rs", ""),
            ("crates/app/src/lib.rs", ""),
            ("crates/app/src/main.rs", ""),
            ("crates/app/src/a.rs", ""),
            ("crates/app/src/a/b.rs", ""),
            ("crates/app/src/c/mod.rs", ""),
            ("crates/app/src/c/d.rs", ""),
            ("crates/app/src/bin/tool.rs", ""),
            ("crates/app/src/bin/multi/main.rs", ""),
            ("crates/app/src/bin/multi/part.rs", ""),
            ("crates/app/tests/it.rs", ""),
            ("crates/app/tests/common/mod.rs", ""),
            ("crates/app/benches/speed.rs", ""),
            ("crates/app/examples/demo.rs", ""),
            ("crates/app/tools/extra.rs", ""),
            ("crates/app/checks/custom.rs", ""),
            ("crates/core-lib/Cargo.toml", include_str!("fixtures/core-lib.toml")),
            ("crates/core-lib/src/lib.rs", ""),
            ("crates/core-lib/src/model.rs", ""),
            ("crates/core-lib/src/store/mod.rs", ""),
            ("crates/util/Cargo.toml", include_str!("fixtures/util.toml")),
            ("crates/util/lib/lib.rs", ""),
            ("crates/util/lib/helper.rs", ""),
        ])
    }

    /// A workspace of two crates that depend on each other, whose root has a
    /// package of its own: `root-app`, which uses `member` through its
    /// workspace, and `member`.
    pub(crate) fn root_package() -> Self {
        Self::new(&[
            ("Cargo.toml", include_str!("fixtures/root-package.toml")),
            ("src/lib.rs", ""),
            ("member/Cargo.toml", include_str!("fixtures/member.toml")),
            ("member/src/lib.rs", ""),
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

    /// The `Cargo.toml` files of the project, sorted.
    pub(crate) fn manifests(&self) -> &[PathBuf] {
        &self.manifests
    }

    /// The crates of the project, found by reading its manifests.
    pub(crate) fn crates(&self) -> Crates {
        Crates::discover(&self.root, &self.manifests, Discovery::Manifests)
    }
}
