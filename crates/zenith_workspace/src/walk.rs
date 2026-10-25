//! Finding the files to scan, adapted from the ruff-derived resolvers in
//! `teamsearch` and `bracketlint`.

use std::{
    mem,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use ignore::{
    DirEntry, Match, ParallelVisitor, ParallelVisitorBuilder, WalkBuilder, WalkState,
    gitignore::{Gitignore, GitignoreBuilder},
};
use log::warn;
use rustc_hash::FxHashMap;
use zenith_utils::{
    fs::{self, FileKind, FileSystem, OsFileSystem},
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
    ".venv",
    "site-packages",
    "__pycache__",
    ".tox",
    ".nox",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".terraform",
    ".terragrunt-cache",
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
    /// `source_extensions` or `source_file_names`.
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

    /// The names of the source files to scan, such as `terragrunt.hcl`,
    /// besides those that match one of the `include` globs.
    pub source_file_names: Vec<&'static str>,

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

/// Walk the project rooted at `root` in `file_system`, which must be
/// canonical there: the files that the file system lists, or else those
/// that walking the disk finds.
///
/// Excluded paths are invisible: their files aren't scanned, and a package
/// whose directory or manifest is excluded isn't found. Symlinks are
/// never followed, and symlinked files are skipped, so each file is found once,
/// at its real path.
pub fn walk(file_system: &dyn FileSystem, root: &Path, settings: &WalkSettings) -> WalkResult {
    match file_system.listed_files(root) {
        Some(files) => walk_listed(file_system, root, files, settings, true),
        None => walk_with(root, settings, true),
    }
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

/// [`walk`] over `files`, those that `file_system` lists under `root`,
/// leaving out what walking them would: the files in the directories that it
/// skips, and, if the settings respect them, those that the ignore files
/// ignore, as [`IgnoreFiles`] reads them, with git's global excludes if
/// `git_global` is set.
fn walk_listed(
    file_system: &dyn FileSystem,
    root: &Path,
    mut files: Vec<fs::DirEntry>,
    settings: &WalkSettings,
    git_global: bool,
) -> WalkResult {
    let ignore_files = (settings.respect_gitignore)
        .then(|| IgnoreFiles::new(file_system, root, &files, git_global));
    let ignored = |path: &Path, is_dir| {
        ignore_files.as_ref().is_some_and(|ignore_files| ignore_files.ignore(path, is_dir))
    };

    files.sort_unstable_by(|a, b| compare_paths(&a.path, &b.path));
    let mut skipped_dirs: FxHashMap<String, bool> = FxHashMap::default();
    let mut result = WalkResult::default();
    for file in files {
        // 1. Symlinks are skipped, as the walk of the disk skips them.
        let Some(relative) = relative_posix(root, &file.path) else {
            continue;
        };
        if file.kind != FileKind::File || relative.is_empty() {
            continue;
        }

        // 2. Skip the files in the directories that the walk skips, deciding on each
        //    directory once, from the root down.
        let mut skipped = false;
        for (end, _) in relative.match_indices('/') {
            let dir = &relative[..end];
            skipped = *skipped_dirs.entry(dir.to_owned()).or_insert_with(|| {
                let path = root.join(dir);
                let name = dir.rsplit('/').next().unwrap_or(dir);
                settings.skips_dir(file_system, dir, name, &path) || ignored(&path, true)
            });
            if skipped {
                break;
            }
        }
        if skipped || ignored(&file.path, false) {
            continue;
        }

        // 3. Collect it, as a manifest or a source file.
        let name = relative.rsplit('/').next().unwrap_or(&relative);
        settings.collect_file(&relative, name, &file.path, &mut result);
    }
    result
}

/// The ignore files that apply to a listed walk, in the order that the walk
/// of the disk applies them, each kind winning over those after it: the
/// `.ignore` files, then the `.gitignore` files, both read from the walk's
/// file system, then the repository's `info/exclude` and git's global
/// excludes, which are on the disk.
struct IgnoreFiles {
    /// The `.ignore` of each directory that has one.
    ignore: FxHashMap<PathBuf, Gitignore>,

    /// The `.gitignore` of each directory that has one.
    gitignore: FxHashMap<PathBuf, Gitignore>,

    /// The repository's `info/exclude` and git's global excludes, if there
    /// are any.
    excludes: Vec<Gitignore>,
}

impl IgnoreFiles {
    /// The ignore files in `file_system` of the walk of `root`: those among
    /// `files`, the files under it, in it, and above it, up to the root of
    /// the repository that it is in, if it is in one. With `git_global`,
    /// git's global excludes apply too.
    fn new(
        file_system: &dyn FileSystem,
        root: &Path,
        files: &[fs::DirEntry],
        git_global: bool,
    ) -> Self {
        // 1. Find the directories that may hold them, and the repository's root.
        let mut dirs = Vec::new();
        let mut repository = None;
        for dir in root.ancestors() {
            dirs.push(dir.to_path_buf());
            if dir.join(".git").exists() {
                repository = Some(dir);
                break;
            }
        }
        let names = [".ignore", ".gitignore"];
        let inside = files.iter().filter(|file| {
            file.path.file_name().is_some_and(|name| names.iter().any(|known| name == *known))
        });
        dirs.extend(inside.filter_map(|file| Some(file.path.parent()?.to_path_buf())));
        dirs.sort_unstable();
        dirs.dedup();

        // 2. Read those in the directories.
        let mut ignore_files = Self {
            ignore: FxHashMap::default(),
            gitignore: FxHashMap::default(),
            excludes: Vec::new(),
        };
        for dir in dirs {
            if let Some(matcher) = matcher(file_system, &dir.join(".ignore"), &dir) {
                ignore_files.ignore.insert(dir.clone(), matcher);
            }
            if let Some(matcher) = matcher(file_system, &dir.join(".gitignore"), &dir) {
                ignore_files.gitignore.insert(dir, matcher);
            }
        }

        // 3. Read the repository's excludes, and git's global ones.
        let exclude = repository
            .and_then(|repository| matcher(&OsFileSystem, &info_exclude(repository)?, repository));
        ignore_files.excludes.extend(exclude);
        if git_global {
            let (global, _) = Gitignore::global();
            ignore_files.excludes.extend((!global.is_empty()).then_some(global));
        }
        ignore_files
    }

    /// Whether the files ignore `path`: the first of them, in the order that
    /// they apply, nearest first, that says anything about it decides.
    fn ignore(&self, path: &Path, is_dir: bool) -> bool {
        let dirs = || path.ancestors().skip(1);
        let matchers = (dirs().filter_map(|dir| self.ignore.get(dir)))
            .chain(dirs().filter_map(|dir| self.gitignore.get(dir)))
            .chain(&self.excludes);
        for matcher in matchers {
            match matcher.matched(path, is_dir) {
                Match::Ignore(_) => return true,
                Match::Whitelist(_) => return false,
                Match::None => {}
            }
        }
        false
    }
}

/// The matcher of the ignore file at `path` in `file_system`, whose patterns
/// are relative to `dir`, or `None` if it can't be read.
fn matcher(file_system: &dyn FileSystem, path: &Path, dir: &Path) -> Option<Gitignore> {
    let contents = file_system.read_text(path).ok()?;
    let mut builder = GitignoreBuilder::new(dir);
    for line in contents.lines() {
        let _ = builder.add_line(Some(path.to_path_buf()), line);
    }
    builder.build().ok()
}

/// The path of the `info/exclude` of the repository whose worktree is at
/// `dir`: in its `.git` directory, or in the common directory of the linked
/// worktree or submodule that its `.git` file points to.
fn info_exclude(dir: &Path) -> Option<PathBuf> {
    let git = dir.join(".git");
    let git_dir = match git.is_dir() {
        true => git,
        false => dir.join(OsFileSystem.read_text(&git).ok()?.strip_prefix("gitdir:")?.trim()),
    };
    let common = match OsFileSystem.read_text(&git_dir.join("commondir")) {
        Ok(common) => git_dir.join(common.trim()),
        Err(_) => git_dir,
    };
    Some(common.join("info/exclude"))
}

impl WalkSettings {
    /// Whether the walk skips the directory `name` in `file_system`, at the
    /// POSIX path `relative` to the root, and at `path`.
    fn skips_dir(
        &self,
        file_system: &dyn FileSystem,
        relative: &str,
        name: &str,
        path: &Path,
    ) -> bool {
        self.exclude.iter().any(|glob| glob.is_match(relative))
            || ALWAYS_EXCLUDED_DIRS.contains(&name)
            || (PACKAGE_OUTPUT_DIRS.contains(&name) && is_package_output(file_system, path))
    }

    /// Collect the file `name`, at the POSIX path `relative` to the root and
    /// at `path`, into `found`, if it is a manifest or a source file that the
    /// walk doesn't exclude.
    fn collect_file(&self, relative: &str, name: &str, path: &Path, found: &mut WalkResult) {
        if self.exclude.iter().any(|glob| glob.is_match(relative))
            || ALWAYS_EXCLUDED_FILES.contains(&name)
        {
            return;
        }
        if self.manifests.contains(&name) {
            found.manifests.push(path.to_path_buf());
        }
        let has_source_extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| self.source_extensions.contains(&extension));
        let included = has_source_extension
            || self.source_file_names.contains(&name)
            || self.include.iter().any(|glob| glob.is_match(relative));
        if included {
            found.source_files.push(path.to_path_buf());
        }
    }
}

/// Whether `dir` in `file_system`, named like a build output directory, sits
/// directly inside a package without being a package itself.
fn is_package_output(file_system: &dyn FileSystem, dir: &Path) -> bool {
    dir.parent().is_some_and(|parent| file_system.is_file(&parent.join("package.json")))
        && !file_system.is_file(&dir.join("package.json"))
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
        let Some(file_type) = entry.file_type() else {
            return WalkState::Continue;
        };

        if file_type.is_dir() {
            let skip = self.settings.skips_dir(&OsFileSystem, &relative, &name, path);
            return if skip { WalkState::Skip } else { WalkState::Continue };
        }

        // This also skips symlinks, which the walk doesn't follow.
        if file_type.is_file() {
            self.settings.collect_file(&relative, &name, path, &mut self.local);
        }
        WalkState::Continue
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, io};

    use tempfile::TempDir;
    use zenith_utils::fs::adjust_canonicalisation;

    use super::*;
    use crate::{
        Git,
        testing::{commit, git},
    };

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
            source_file_names: Vec::new(),
            manifests: vec!["package.json"],
        }
    }

    /// The disk, which lists every file on it, as a commit's tree lists its
    /// files, so that its walk is the walk of listed files.
    #[derive(Debug)]
    struct Listed;

    impl Listed {
        /// Add the files under `dir`, without following links, to `files`.
        fn add(dir: &Path, files: &mut Vec<zenith_utils::fs::DirEntry>) {
            for entry in OsFileSystem.read_dir(dir).unwrap() {
                match entry.kind {
                    FileKind::Directory => Self::add(&entry.path, files),
                    _ => files.push(entry),
                }
            }
        }
    }

    impl FileSystem for Listed {
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            OsFileSystem.read(path)
        }

        fn kind(&self, path: &Path) -> io::Result<FileKind> {
            OsFileSystem.kind(path)
        }

        fn link_kind(&self, path: &Path) -> io::Result<FileKind> {
            OsFileSystem.link_kind(path)
        }

        fn read_dir(&self, path: &Path) -> io::Result<Vec<zenith_utils::fs::DirEntry>> {
            OsFileSystem.read_dir(path)
        }

        fn read_link(&self, path: &Path) -> io::Result<PathBuf> {
            OsFileSystem.read_link(path)
        }

        fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
            OsFileSystem.canonicalize(path)
        }

        fn listed_files(&self, dir: &Path) -> Option<Vec<zenith_utils::fs::DirEntry>> {
            let mut files = Vec::new();
            Self::add(dir, &mut files);
            Some(files)
        }
    }

    /// The source files and `package.json` files found, relative to `root`,
    /// which the walk of the disk and the walk of its listed files must agree
    /// on. Git's global excludes are off, so the developer's configuration
    /// can't leak in.
    fn found(root: &Path, settings: &WalkSettings) -> (Vec<String>, Vec<String>) {
        let relative = |result: WalkResult| -> (Vec<String>, Vec<String>) {
            let relative = |paths: &[PathBuf]| -> Vec<String> {
                paths.iter().map(|path| relative_posix(root, path).unwrap()).collect()
            };
            (relative(&result.source_files), relative(&result.manifests))
        };
        let walked = relative(walk_with(root, settings, false));
        let files = Listed.listed_files(root).unwrap();
        let listed = relative(walk_listed(&Listed, root, files, settings, false));
        assert_eq!(listed, walked, "the walk of the listed files differs");
        walked
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
            "infra/.terraform/modules/vpc/main.ts",
            "infra/.terragrunt-cache/x/main.ts",
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

    #[test]
    fn source_file_names_select_files_by_name() {
        let (_dir, root) = project(&["live/terragrunt.hcl", "live/backend.hcl", "src/a.ts"]);

        let names = WalkSettings { source_file_names: vec!["terragrunt.hcl"], ..settings(&[]) };
        let (files, _) = found(&root, &names);
        assert_eq!(files, ["live/terragrunt.hcl", "src/a.ts"]);
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

    #[test]
    fn the_excludes_of_the_repository_apply() {
        let (_dir, root) = project(&["src/a.ts", "src/b.ts"]);
        write(&root, ".git/info/exclude", "b.ts\n");
        assert_eq!(found(&root, &settings(&[])).0, ["src/a.ts"]);
    }

    #[test]
    fn every_ignore_file_wins_over_every_gitignore() {
        let (_dir, root) = project(&["src/a.ts", "src/b.ts"]);
        fs::create_dir(root.join(".git")).unwrap();
        write(&root, ".ignore", "!a.ts\n");
        write(&root, "src/.gitignore", "*.ts\n");
        assert_eq!(found(&root, &settings(&[])).0, ["src/a.ts"]);
    }

    #[test]
    fn a_commit_is_walked_from_its_listed_files_as_the_disk_is() {
        let (_dir, root) = project(&["package.json", "src/a.ts", "dist/b.ts", "src/c.ts"]);
        write(&root, ".gitignore", "generated/\n");
        write(&root, "generated/d.ts", "");
        git(&root, &["init", "--quiet"]);
        // A file that `.gitignore` ignores, even though it was committed.
        git(&root, &["add", "--force", "generated/d.ts"]);
        commit(&root);
        // Changes since the commit, which its walk doesn't see.
        fs::remove_file(root.join("src/c.ts")).unwrap();
        write(&root, "src/e.ts", "");

        let files = Git::new(&root).commit_files("HEAD").unwrap();
        let result = walk(&files, &root, &settings(&[]));
        let relative = |paths: &[PathBuf]| -> Vec<String> {
            paths.iter().map(|path| relative_posix(&root, path).unwrap()).collect()
        };
        assert_eq!(relative(&result.source_files), ["src/a.ts", "src/c.ts"]);
        assert_eq!(relative(&result.manifests), ["package.json"]);

        let mut keep_ignored = settings(&[]);
        keep_ignored.respect_gitignore = false;
        let result = walk(&files, &root, &keep_ignored);
        assert_eq!(relative(&result.source_files), ["generated/d.ts", "src/a.ts", "src/c.ts"]);
    }
}
