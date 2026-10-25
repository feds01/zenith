//! Finding the crates by reading the `Cargo.toml` files, for when Cargo can't
//! say.

use std::path::Path;

use rustc_hash::FxHashMap;
use zenith_utils::fs::FileSystem;

use super::{Crates, Found, TargetKind, edition, manifest::Manifest};

impl Crates {
    /// The crates of the packages in `manifests`, each with its directory in
    /// `file_system`.
    pub(crate) fn from_manifests(
        file_system: &dyn FileSystem,
        manifests: &[(&Path, Manifest)],
    ) -> Self {
        // The workspaces declare the dependencies that their members share.
        let workspaces: FxHashMap<&Path, &Manifest> = manifests
            .iter()
            .filter(|(_, manifest)| manifest.workspace.is_some())
            .map(|(dir, manifest)| (*dir, manifest))
            .collect();
        let found = manifests
            .iter()
            .filter_map(|(dir, manifest)| {
                let package = manifest.package.as_ref()?;
                let workspace = dir.ancestors().find_map(|dir| Some((dir, *workspaces.get(dir)?)));
                let targets = manifest.targets(file_system, dir);
                let has_library = targets.iter().any(|target| target.kind == TargetKind::Lib);
                Some(Found {
                    name: package.name.clone(),
                    dir: dir.to_path_buf(),
                    edition: edition(manifest.edition(workspace.map(|(_, manifest)| manifest))),
                    library: has_library.then(|| manifest.library_name()),
                    targets,
                    dependencies: manifest.declared(dir, workspace),
                })
            })
            .collect();
        Self::new(found)
    }
}
