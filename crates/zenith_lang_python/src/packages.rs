//! The packages of a project, each from its `pyproject.toml`.

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use log::warn;
use serde::Deserialize;
use zenith_lang::{Workspace, WorkspacePackage};
use zenith_utils::{fs::read_text, path::relative_posix};

/// A package of the project: a directory with a `pyproject.toml` that names
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Package {
    /// The name of the package, as its `pyproject.toml` writes it.
    pub(crate) name: String,

    /// Its directory, as an absolute path.
    pub(crate) dir: PathBuf,
}

impl Package {
    /// The packages in the directories of `manifests`, the `pyproject.toml`
    /// files of the project, sorted by directory. A manifest that names no
    /// package, such as one that only configures tools, is left out, and one
    /// that can't be read is too, with a warning.
    pub(crate) fn read_all(manifests: &[PathBuf]) -> Vec<Package> {
        let mut packages: Vec<_> = manifests
            .iter()
            .filter_map(|path| {
                let manifest = read_text(path)
                    .map_err(|error| error.to_string())
                    .and_then(|text| toml::from_str(&text).map_err(|error| error.to_string()));
                let manifest: Manifest = match manifest {
                    Ok(manifest) => manifest,
                    Err(error) => {
                        warn!("could not read `{}`: {error}", path.display());
                        return None;
                    }
                };
                Some(Package { name: manifest.name()?, dir: path.parent()?.to_path_buf() })
            })
            .collect();
        packages.sort_by(|a, b| a.dir.cmp(&b.dir));
        packages
    }

    /// The directory that the absolute imports of its modules start from:
    /// its `src` directory in the src layout, and its own directory
    /// otherwise.
    pub(crate) fn root(&self) -> PathBuf {
        let src = self.dir.join("src");
        if src.is_dir() { src } else { self.dir.clone() }
    }

    /// `packages` as the workspace packages of the project at `root`, which
    /// is the root package if one is in it.
    pub(crate) fn workspace(root: &Path, packages: &[Package]) -> Workspace {
        let mut workspace = Workspace::default();
        for package in packages {
            let Some(dir) = relative_posix(root, &package.dir) else {
                continue;
            };
            let found =
                WorkspacePackage { name: package.name.clone(), dir, path: package.dir.clone() };
            match found.dir.is_empty() {
                true => workspace.root_package = Some(found),
                false => workspace.packages.push(found),
            }
        }
        workspace
    }
}

/// A `pyproject.toml`, as far as zenith reads it. Everything else in it is
/// left unread.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Manifest {
    /// The `[project]` table of the packaging standard.
    project: Option<Named>,

    /// The `[tool]` table.
    tool: Option<Tool>,
}

impl Manifest {
    /// The name of the package: that of `[project]`, or failing that, that of
    /// `[tool.poetry]`, which Poetry read before it followed the standard.
    fn name(self) -> Option<String> {
        let poetry = self.tool.and_then(|tool| tool.poetry);
        self.project.and_then(|project| project.name).or(poetry.and_then(|poetry| poetry.name))
    }
}

/// The `[tool]` table of a `pyproject.toml`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Tool {
    /// The `[tool.poetry]` table.
    poetry: Option<Named>,
}

/// A table with the name of the package.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Named {
    /// The name of the package.
    name: Option<String>,
}
