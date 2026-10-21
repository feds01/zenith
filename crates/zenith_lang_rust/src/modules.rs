//! The module tree of a crate: where each file sits in it, and which file
//! each module is.

use std::path::{Path, PathBuf};

use zenith_utils::path::relative_posix;

use crate::crates::Crate;

/// The root module of a crate, from which the tree of its modules is read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Root {
    /// The file of the root module, such as `src/lib.rs`. `None` for the
    /// files under `tests`, `benches` and `examples`, which belong to every
    /// crate that those directories hold.
    file: Option<PathBuf>,

    /// The directory that the files of its submodules are in.
    dir: PathBuf,
}

/// Where a file sits in the module tree of its crate.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Position {
    /// The root of its tree.
    pub(crate) root: Root,

    /// The names of the modules from the root to the file's module, which are
    /// none for the root itself.
    pub(crate) modules: Vec<String>,
}

impl Root {
    /// The file of the module that `modules`, the names of the modules from
    /// the root, name, and how many of them it takes to name it. A module is
    /// in `a.rs` or `a/mod.rs`, in the directory of its parent. Only the
    /// longest prefix of `modules` that is a file counts, as the rest are
    /// items in it, or modules that it declares inline. With no such prefix,
    /// the module is the root.
    pub(crate) fn module_file(&self, modules: &[&str]) -> Option<(PathBuf, usize)> {
        let mut dir = self.dir.clone();
        let mut found = None;
        for (depth, module) in modules.iter().enumerate() {
            dir.push(module);
            let files = [dir.with_extension("rs"), dir.join("mod.rs")];
            let Some(file) = files.into_iter().find(|file| file.is_file()) else {
                break;
            };
            found = Some((file, depth + 1));
        }
        found.or_else(|| Some((self.file.clone()?, 0)).filter(|(file, _)| file.is_file()))
    }
}

impl Crate {
    /// The root of its library.
    pub(crate) fn library(&self) -> Root {
        let file = self.dir.join(&self.lib_path);
        Root { dir: file.parent().unwrap_or(&self.dir).to_path_buf(), file: Some(file) }
    }

    /// Where the file at `path` sits in the module tree of the crate. `None`
    /// if it is in none. A crate has a tree for each of its library, its
    /// binaries, its build script, its tests, its benchmarks and its examples.
    /// The file of a library or a binary is in the tree of the library if it
    /// is under its directory, and otherwise in that of `src/main.rs`.
    pub(crate) fn locate(&self, path: &Path) -> Option<Position> {
        let rel = relative_posix(&self.dir, path)?;
        let segments: Vec<&str> = rel.split('/').collect();

        // 1. A root file is the root of a tree of its own.
        let is_root = match segments.as_slice() {
            ["build.rs"] | ["src", "main.rs"] | ["src", "bin", _, "main.rs"] => true,
            ["src", "bin", file] | ["tests" | "benches" | "examples", file] => {
                file.ends_with(".rs")
            }
            _ => rel == self.lib_path,
        };
        if is_root {
            let root = Root { file: Some(path.to_path_buf()), dir: path.parent()?.to_path_buf() };
            return Some(Position { root, modules: Vec::new() });
        }

        // 2. Any other file is in the tree of the nearest directory that holds one,
        //    which is the tree of the library if it is there. A tree is its directory,
        //    and the name of the file of its root in it, if it has one.
        let mut trees: Vec<(&str, Option<&str>)> = Vec::new();
        if self.dir.join(&self.lib_path).is_file() {
            let (dir, name) = self.lib_path.rsplit_once('/').unwrap_or(("", &self.lib_path));
            trees.push((dir, Some(name)));
        }
        trees.push(("src", Some("main.rs")));
        match segments.as_slice() {
            ["src", "bin", name, _, ..] => {
                trees.push((&rel[.."src/bin/".len() + name.len()], Some("main.rs")))
            }
            [dir @ ("tests" | "benches" | "examples"), _, _, ..] => trees.push((dir, None)),
            _ => {}
        }
        let (dir, file) = (trees.into_iter().filter(|(dir, _)| within(&rel, dir)))
            .reduce(|best, next| if next.0.len() > best.0.len() { next } else { best })?;
        let mut modules: Vec<String> = rel[dir.len()..]
            .strip_suffix(".rs")?
            .split('/')
            .filter(|module| !module.is_empty())
            .map(str::to_owned)
            .collect();
        if modules.last().is_some_and(|module| module == "mod") {
            modules.pop();
        }
        let dir = self.dir.join(dir);
        let root = Root { file: file.map(|file| dir.join(file)), dir };
        Some(Position { root, modules })
    }
}

/// Whether the POSIX path `path` is inside the directory `dir`, which is empty
/// for the directory of the root.
fn within(path: &str, dir: &str) -> bool {
    dir.is_empty() || path.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/'))
}
