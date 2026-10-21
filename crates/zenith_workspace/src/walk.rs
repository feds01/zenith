//! Finding the files to scan, adapted from the ruff-derived resolvers in
//! `teamsearch` and `bracketlint`.

use std::{
    mem,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use ignore::{DirEntry, ParallelVisitor, ParallelVisitorBuilder, WalkBuilder, WalkState};
use log::warn;
use zenith_utils::{
    glob::Glob,
    path::{compare_paths, relative_posix},
    pool::thread_count,
};

/// Directories that are never scanned, wherever they appear.
const ALWAYS_EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    ".yarn",
    ".next",
    ".nuxt",
    ".output",
    ".svelte-kit",
    ".turbo",
    ".vercel",
    ".cache",
    ".wxt",
    ".mastra",
    "coverage",
    "storybook-static",
];

/// Files that are never scanned: Yarn Plug'n'Play's generated loaders.
const ALWAYS_EXCLUDED_FILES: &[&str] = &[".pnp.cjs", ".pnp.loader.mjs"];

/// Build output directories, which are skipped when they sit directly inside
/// a package (next to a `package.json`). Elsewhere, e.g. `src/build/`, they
/// may well be source code.
const PACKAGE_OUTPUT_DIRS: &[&str] = &["dist", "build", "out"];

/// Settings for [`walk`].
#[derive(Debug, Clone)]
pub struct WalkSettings {
    /// Globs of the files to scan, besides those with one of the
    /// `source_extensions`.
    pub include: Vec<Glob>,

    /// Globs of the files and directories to skip.
    pub exclude: Vec<Glob>,

    /// Whether to respect `.gitignore`, `.ignore` and git's global and
    /// repository excludes.
    pub respect_gitignore: bool,

    /// The extensions, without the `.`, of the source files to scan, besides
    /// those that match one of the `include` globs. Checking an extension is
    /// cheaper than matching globs.
    pub source_extensions: Vec<&'static str>,

    /// The names of the manifest files to collect, such as `package.json`,
    /// from which the languages find the project's packages.
    pub manifests: Vec<&'static str>,
}

/// The files found by [`walk`], as absolute paths, sorted.
#[derive(Debug, Clone, Default)]
pub struct WalkResult {
    /// The source files to scan.
    pub source_files: Vec<PathBuf>,

    /// Every manifest file, as [`WalkSettings::manifests`] names them, that
    /// isn't excluded, for finding the project's packages.
    pub manifests: Vec<PathBuf>,
}

/// Walk the project rooted at `root`, which must be canonical.
///
/// Excluded paths are invisible: their files aren't scanned, and a package
/// whose directory or manifest is excluded isn't found. Symlinks are
/// never followed, and symlinked files are skipped, so each file is found once,
/// at its real path.
pub fn walk(root: &Path, settings: &WalkSettings) -> WalkResult {
    walk_with(root, settings, true)
}

/// [`walk`], choosing whether git's global excludes apply, so that tests don't
/// depend on the developer's git configuration.
fn walk_with(root: &Path, settings: &WalkSettings, git_global: bool) -> WalkResult {
    let respect = settings.respect_gitignore;
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .parents(respect)
        .ignore(respect)
        .git_ignore(respect)
        .git_global(respect && git_global)
        .git_exclude(respect)
        // Inside a repository, ignore files above the repository don't apply.
        // Outside one, `.gitignore` files still do.
        .require_git(in_git_repository(root))
        .threads(thread_count())
        .build_parallel();

    let result = Mutex::new(WalkResult::default());
    walker.visit(&mut VisitorBuilder { root, settings, result: &result });

    let mut result = result.into_inner().unwrap_or_else(PoisonError::into_inner);
    // Each path is found once, and comparing them by components would cost more
    // than the rest of the walk's own work.
    result.source_files.sort_unstable_by(|a, b| compare_paths(a, b));
    result.manifests.sort_unstable_by(|a, b| compare_paths(a, b));
    result
}

/// Whether `dir` is inside a git repository or worktree, whose `.git` is a
/// directory or a file.
fn in_git_repository(dir: &Path) -> bool {
    dir.ancestors().any(|dir| dir.join(".git").exists())
}

/// Whether `dir`, named like a build output directory, sits directly inside a
/// package without being a package itself.
fn is_package_output(dir: &Path) -> bool {
    dir.parent().is_some_and(|parent| parent.join("package.json").is_file())
        && !dir.join("package.json").is_file()
}

/// Builds one [`Visitor`] per walker thread.
struct VisitorBuilder<'a> {
    /// The canonical project root.
    root: &'a Path,

    /// The walk's settings.
    settings: &'a WalkSettings,

    /// The result that every visitor merges into.
    result: &'a Mutex<WalkResult>,
}

impl<'s> ParallelVisitorBuilder<'s> for VisitorBuilder<'s> {
    fn build(&mut self) -> Box<dyn ParallelVisitor + 's> {
        Box::new(Visitor {
            root: self.root,
            settings: self.settings,
            local: WalkResult::default(),
            result: self.result,
        })
    }
}

/// A per-thread visitor that collects files locally, and merges them into the
/// shared result when its thread finishes.
struct Visitor<'a> {
    /// The canonical project root.
    root: &'a Path,

    /// The walk's settings.
    settings: &'a WalkSettings,

    /// The files this thread found.
    local: WalkResult,

    /// The result to merge into.
    result: &'a Mutex<WalkResult>,
}

impl Drop for Visitor<'_> {
    fn drop(&mut self) {
        let local = mem::take(&mut self.local);
        let mut result = self.result.lock().unwrap_or_else(PoisonError::into_inner);
        result.source_files.extend(local.source_files);
        result.manifests.extend(local.manifests);
    }
}

impl ParallelVisitor for Visitor<'_> {
    fn visit(&mut self, entry: Result<DirEntry, ignore::Error>) -> WalkState {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warn!("{error}");
                return WalkState::Continue;
            }
        };

        let path = entry.path();
        let Some(relative) = relative_posix(self.root, path) else {
            return WalkState::Continue;
        };
        if relative.is_empty() {
            return WalkState::Continue;
        }

        let name = entry.file_name().to_string_lossy();
        let excluded = self.settings.exclude.iter().any(|glob| glob.is_match(&relative));
        let Some(file_type) = entry.file_type() else {
            return WalkState::Continue;
        };

        if file_type.is_dir() {
            let skip = excluded
                || ALWAYS_EXCLUDED_DIRS.contains(&name.as_ref())
                || (PACKAGE_OUTPUT_DIRS.contains(&name.as_ref()) && is_package_output(path));
            return if skip { WalkState::Skip } else { WalkState::Continue };
        }

        // This also skips symlinks, which the walk doesn't follow.
        if excluded || !file_type.is_file() || ALWAYS_EXCLUDED_FILES.contains(&name.as_ref()) {
            return WalkState::Continue;
        }

        if self.settings.manifests.contains(&name.as_ref()) {
            self.local.manifests.push(path.to_path_buf());
        }

        let has_source_extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| self.settings.source_extensions.contains(&extension));
        let included = has_source_extension
            || self.settings.include.iter().any(|glob| glob.is_match(&relative));
        if included {
            self.local.source_files.push(path.to_path_buf());
        }

        WalkState::Continue
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use zenith_utils::path::adjust_canonicalisation;

    use super::*;

    /// A temporary project with the given files, and its canonical root.
    fn project(files: &[&str]) -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        for file in files {
            write(&root, file, "");
        }
        (dir, root)
    }

    fn write(root: &Path, file: &str, contents: &str) {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn settings(exclude: &[&str]) -> WalkSettings {
        WalkSettings {
            include: Vec::new(),
            exclude: exclude.iter().map(|glob| Glob::new(glob).unwrap()).collect(),
            respect_gitignore: true,
            source_extensions: vec!["ts", "tsx", "js", "cjs", "mjs"],
            manifests: vec!["package.json"],
        }
    }

    /// The source files and `package.json` files found, relative to `root`.
    /// Git's global excludes are off, so the developer's configuration can't
    /// leak in.
    fn found(root: &Path, settings: &WalkSettings) -> (Vec<String>, Vec<String>) {
        let result = walk_with(root, settings, false);
        let relative = |paths: &[PathBuf]| -> Vec<String> {
            paths.iter().map(|path| relative_posix(root, path).unwrap()).collect()
        };
        (relative(&result.source_files), relative(&result.manifests))
    }

    #[test]
    fn skips_default_and_configured_excludes() {
        let (_dir, root) = project(&[
            "package.json",
            "web/package.json",
            "web/src/a.ts",
            "web/src/build/b.ts",
            "web/dist/c.js",
            "web/node_modules/x/index.js",
            "web/.next/d.js",
            "web/src/routes.gen.ts",
            "web/.storybook/main.ts",
            "experimental/f.ts",
            "out/g.js",
            ".yarn/releases/yarn-4.5.0.cjs",
            ".pnp.cjs",
            "README.md",
        ]);

        let (files, packages) = found(&root, &settings(&["experimental/**", "**/*.gen.ts"]));
        assert_eq!(files, ["web/.storybook/main.ts", "web/src/a.ts", "web/src/build/b.ts"]);
        assert_eq!(packages, ["package.json", "web/package.json"]);
    }

    #[test]
    fn walks_output_directories_that_are_packages() {
        let (_dir, root) = project(&["package.json", "build/package.json", "build/a.ts"]);

        let (files, packages) = found(&root, &settings(&[]));
        assert_eq!(files, ["build/a.ts"]);
        assert_eq!(packages, ["build/package.json", "package.json"]);
    }

    #[test]
    fn excluded_directories_hide_their_packages() {
        let (_dir, root) = project(&[
            "packages/legacy/package.json",
            "packages/legacy/a.ts",
            "packages/old/package.json",
            "packages/old/b.ts",
            "packages/web/package.json",
        ]);

        let exclude = ["packages/legacy", "packages/old/package.json"];
        let (files, packages) = found(&root, &settings(&exclude));
        assert_eq!(files, ["packages/old/b.ts"]);
        assert_eq!(packages, ["packages/web/package.json"]);
    }

    #[test]
    fn respects_ignore_files_without_a_repository() {
        let (_dir, root) = project(&[".worktrees/copy/a.ts", "scratch/b.ts", "src/c.ts"]);
        write(&root, ".gitignore", ".worktrees/\n");
        write(&root, ".ignore", "scratch/\n");

        assert_eq!(found(&root, &settings(&[])).0, ["src/c.ts"]);

        let settings = WalkSettings { respect_gitignore: false, ..settings(&[]) };
        assert_eq!(found(&root, &settings).0, [".worktrees/copy/a.ts", "scratch/b.ts", "src/c.ts"]);
    }

    #[test]
    fn ignore_files_above_a_repository_do_not_apply() {
        let (_dir, outer) = project(&["project/src/a.ts"]);
        write(&outer, ".gitignore", "*.ts\n");
        let root = outer.join("project");
        fs::create_dir(root.join(".git")).unwrap();

        assert_eq!(found(&root, &settings(&[])).0, ["src/a.ts"]);
    }

    #[test]
    fn include_globs_select_files_without_the_extensions() {
        let (_dir, root) = project(&["src/a.ts", "src/b.vue", "scripts/c.ts"]);

        let include = vec![Glob::new("src/**").unwrap()];
        let globs_only = WalkSettings { include, source_extensions: Vec::new(), ..settings(&[]) };
        let (files, _) = found(&root, &globs_only);
        assert_eq!(files, ["src/a.ts", "src/b.vue"]);

        // With the extensions too, the files of either kind are scanned.
        let both = WalkSettings { include: globs_only.include, ..settings(&[]) };
        let (files, _) = found(&root, &both);
        assert_eq!(files, ["scripts/c.ts", "src/a.ts", "src/b.vue"]);
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinks() {
        use std::os::unix::fs::symlink;

        let (_dir, root) = project(&["shared/x.ts", "src/a.ts"]);
        symlink(root.join("shared/x.ts"), root.join("src/linked.ts")).unwrap();
        symlink(root.join("shared"), root.join("src/linked_dir")).unwrap();
        symlink(root.join("missing.ts"), root.join("src/broken.ts")).unwrap();

        let include = vec![Glob::new("**").unwrap()];
        let all = WalkSettings { include, source_extensions: Vec::new(), ..settings(&[]) };
        let (files, _) = found(&root, &all);
        assert_eq!(files, ["shared/x.ts", "src/a.ts"]);
    }
}
