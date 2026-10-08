//! Finding the files to scan, adapted from the ruff-derived resolvers in
//! `teamsearch` and `bracketlint`.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    thread::available_parallelism,
};

use ignore::{WalkBuilder, WalkState};
use zenith_utils::{glob::Glob, path::relative_posix};

use crate::{ALWAYS_EXCLUDED_DIRS, PACKAGE_OUTPUT_DIRS};

/// Settings for [walk].
#[derive(Debug, Clone)]
pub struct WalkSettings {
    /// Globs of files to scan. `None` scans every file with a source
    /// extension.
    pub include: Option<Vec<Glob>>,
    /// Globs of files and directories to skip.
    pub exclude: Vec<Glob>,
    /// Whether to respect `.gitignore` and friends.
    pub respect_gitignore: bool,
    /// The extensions of source files, used when `include` is `None`.
    pub source_extensions: &'static [&'static str],
}

/// The files found by [walk], as absolute paths, sorted.
#[derive(Debug, Clone, Default)]
pub struct WalkResult {
    /// The source files to parse.
    pub source_files: Vec<PathBuf>,
    /// Every `package.json`, for workspace package discovery.
    pub package_jsons: Vec<PathBuf>,
}

/// Walk the project rooted at `root` (which must be canonical).
pub fn walk(root: &Path, settings: &WalkSettings) -> WalkResult {
    let respect = settings.respect_gitignore;
    let threads = available_parallelism().map_or(1, |n| n.get()).min(12);
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .parents(respect)
        .ignore(respect)
        .git_ignore(respect)
        .git_global(respect)
        .git_exclude(respect)
        .require_git(false)
        .threads(threads)
        .build_parallel();

    let result = Mutex::new(WalkResult::default());

    walker.visit(&mut VisitorBuilder { root, settings, result: &result });

    let mut result = result.into_inner().unwrap();
    result.source_files.sort();
    result.package_jsons.sort();
    result
}

/// Builds one [Visitor] per walker thread.
struct VisitorBuilder<'a> {
    root: &'a Path,
    settings: &'a WalkSettings,
    result: &'a Mutex<WalkResult>,
}

impl<'s> ignore::ParallelVisitorBuilder<'s> for VisitorBuilder<'s> {
    fn build(&mut self) -> Box<dyn ignore::ParallelVisitor + 's> {
        Box::new(Visitor {
            root: self.root,
            settings: self.settings,
            local: Some(WalkResult::default()),
            result: self.result,
        })
    }
}

/// A per-thread visitor that collects files locally and merges them into the
/// shared result when the thread finishes.
struct Visitor<'a> {
    root: &'a Path,
    settings: &'a WalkSettings,
    local: Option<WalkResult>,
    result: &'a Mutex<WalkResult>,
}

impl Drop for Visitor<'_> {
    fn drop(&mut self) {
        if let Some(local) = self.local.take() {
            let mut result = self.result.lock().unwrap();
            result.source_files.extend(local.source_files);
            result.package_jsons.extend(local.package_jsons);
        }
    }
}

impl ignore::ParallelVisitor for Visitor<'_> {
    fn visit(&mut self, entry: Result<ignore::DirEntry, ignore::Error>) -> WalkState {
        let Ok(entry) = entry else {
            return WalkState::Continue;
        };
        let path = entry.path();
        let Some(relative) = relative_posix(self.root, path) else {
            return WalkState::Continue;
        };
        if relative.is_empty() {
            return WalkState::Continue;
        }

        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        let name = entry.file_name().to_string_lossy();

        if is_dir {
            if ALWAYS_EXCLUDED_DIRS.contains(&name.as_ref()) {
                return WalkState::Skip;
            }
            if PACKAGE_OUTPUT_DIRS.contains(&name.as_ref())
                && path.parent().is_some_and(|parent| parent.join("package.json").is_file())
            {
                return WalkState::Skip;
            }
            if self.settings.exclude.iter().any(|glob| glob.is_match(&relative)) {
                return WalkState::Skip;
            }
            return WalkState::Continue;
        }

        let local = self.local.as_mut().unwrap();
        if name == "package.json" {
            local.package_jsons.push(path.to_path_buf());
        }

        if self.settings.exclude.iter().any(|glob| glob.is_match(&relative)) {
            return WalkState::Continue;
        }

        let included = match &self.settings.include {
            Some(include) => include.iter().any(|glob| glob.is_match(&relative)),
            None => path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| self.settings.source_extensions.contains(&extension)),
        };
        if included {
            local.source_files.push(path.to_path_buf());
        }

        WalkState::Continue
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn walks_respecting_excludes_and_gitignore() {
        let dir = std::env::temp_dir().join(format!("zenith-walk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let root = {
            fs::create_dir_all(&dir).unwrap();
            zenith_utils::path::adjust_canonicalisation(&dir)
        };
        let write = |path: &str| {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        };
        fs::write(root.join(".gitignore"), ".worktrees/\n").unwrap();
        write("package.json");
        write("web/package.json");
        write("web/src/a.ts");
        write("web/src/build/b.ts");
        write("web/dist/c.js");
        write("web/node_modules/x/index.js");
        write("web/.next/d.js");
        write("web/src/routes.gen.ts");
        write("web/.storybook/main.ts");
        write(".worktrees/copy/e.ts");
        write("experimental/f.ts");
        write("README.md");

        let settings = WalkSettings {
            include: None,
            exclude: vec![Glob::new("experimental/**").unwrap(), Glob::new("**/*.gen.ts").unwrap()],
            respect_gitignore: true,
            source_extensions: &["ts", "tsx", "js"],
        };
        let result = walk(&root, &settings);
        let files: Vec<_> =
            result.source_files.iter().map(|p| relative_posix(&root, p).unwrap()).collect();
        assert_eq!(files, vec!["web/.storybook/main.ts", "web/src/a.ts", "web/src/build/b.ts"]);
        let packages: Vec<_> =
            result.package_jsons.iter().map(|p| relative_posix(&root, p).unwrap()).collect();
        assert_eq!(packages, vec!["package.json", "web/package.json"]);

        let _ = fs::remove_dir_all(&dir);
    }
}
