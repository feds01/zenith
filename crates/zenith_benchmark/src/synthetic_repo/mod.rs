//! A synthetic TypeScript monorepo, generated deterministically at any size.
//!
//! [`SyntheticRepo::new`] plans a bun-style workspace of `@acme/...`
//! packages, with libraries in `packages/`, services in `services/` and apps
//! in `apps/`, and fills each one with feature directories of components,
//! hooks, API clients, utilities, handlers, jobs, stores and tests, behind
//! barrel `index.ts` files. The files import each other with a realistic
//! density and mix of relative, `tsconfig.json` alias, workspace package, npm
//! and Node.js builtin specifiers. The repository also holds what the walk
//! must skip: `node_modules`, packages' `dist` output, Next.js `.next` builds
//! and a `.gitignore`d directory.
//!
//! Every choice comes from a fixed-seed generator, so a given size always
//! produces the same files, byte for byte.
//!
//! [`plan`] lays out the packages, features and source files, [`generator`]
//! fills in their contents, and [`templates`] holds the fixed text.

mod generator;
mod plan;
mod templates;

use std::{collections::BTreeSet, fs, io, path::Path};

use fastrand::Rng;
use zenith_graph::ImportKind;

/// The fewest source files that [`SyntheticRepo::new`] accepts.
pub const MIN_SOURCE_FILES: usize = 100;

/// The seed of every repository. Changing it, or a `fastrand` update that
/// changes its output, changes every benchmark's input.
const SEED: u64 = 0x2E41_7A11_5EED_0001;

/// A synthetic TypeScript monorepo: every file's path and contents, and what
/// zenith should make of them.
///
/// The repository is generated in memory by [`SyntheticRepo::new`], and
/// written to disk by [`SyntheticRepo::write`]. [`crate::TempRepo`] does both
/// in a temporary directory.
///
/// ```
/// use zenith_benchmark::{ImportTarget, SyntheticRepo};
///
/// let repo = SyntheticRepo::new(1_000);
///
/// // An import of a file of the repository records which file it is.
/// let import = repo.imports().find(|import| import.target == ImportTarget::Alias).unwrap();
/// let file = import.resolves_to.as_deref().unwrap();
/// assert!(repo.files().iter().any(|candidate| candidate.path == file));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticRepo {
    /// Every file, sorted by path.
    files: Vec<SyntheticFile>,

    /// The workspace packages, sorted by directory.
    packages: Vec<SyntheticPackage>,

    /// The names of the npm packages that the source files import, sorted.
    externals: Vec<String>,
}

/// One file of a [`SyntheticRepo`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticFile {
    /// The file's path, as a POSIX path relative to the root.
    pub path: String,

    /// The file's contents.
    pub contents: String,

    /// What zenith's walk should do with the file.
    pub kind: FileKind,

    /// The imports in the file, in source order, including re-exports. Only
    /// source files have any.
    pub imports: Vec<SyntheticImport>,
}

/// What zenith's walk, with its default settings, should do with a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileKind {
    /// A source file, which the walk returns for scanning.
    Source,

    /// A workspace `package.json`, which the walk returns for package
    /// discovery.
    Manifest,

    /// Another file that the walk visits but doesn't return, such as a
    /// `tsconfig.json`, a stylesheet or a README.
    Other,

    /// A file that the walk must skip, in `node_modules`, a package's `dist`,
    /// a `.next` build or a `.gitignore`d directory.
    Skipped,
}

/// A workspace package of a [`SyntheticRepo`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticPackage {
    /// The `name` in its `package.json`, e.g. `@acme/design-system`.
    pub name: String,

    /// Its directory, as a POSIX path relative to the root, e.g.
    /// `packages/designSystem`. Library directories are camel-cased, so they
    /// differ from the package name when it has a `-`.
    pub dir: String,

    /// What the package is for.
    pub role: PackageRole,

    /// Whether its `package.json` has an `exports` map. Packages without one
    /// are imported by deep `src/...` paths as well as by name.
    pub has_exports: bool,

    /// The names of the workspace packages that it depends on, and may
    /// import, sorted.
    pub dependencies: Vec<String>,
}

/// What a workspace package is for. Libraries only depend on libraries that
/// come before them, and services and apps only on libraries, so the package
/// dependency graph has no cycles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PackageRole {
    /// A shared library in `packages/`, with a barrel `src/index.ts`.
    Library,

    /// A Node.js service in `services/`, written as ES modules with `.js`
    /// extensions in its relative imports, plus a CommonJS script.
    Service,

    /// A Next.js app in `apps/`, with route groups and `[id]` routes.
    App,
}

/// An import, or re-export, in a source file of a [`SyntheticRepo`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticImport {
    /// The specifier as written, e.g. `../hooks/use-query` or `@acme/ui`.
    pub specifier: String,

    /// What the specifier refers to.
    pub target: ImportTarget,

    /// How the import is written.
    pub kind: ImportKind,

    /// The file that the specifier resolves to, as a POSIX path relative to
    /// the root, or `None` for npm packages and builtins.
    pub resolves_to: Option<String>,
}

/// What an import's specifier refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ImportTarget {
    /// A relative path, e.g. `./list` or `../db/table.js`.
    Relative,

    /// A `tsconfig.json` `paths` alias, e.g. `@/features/billing`.
    Alias,

    /// Another workspace package, by name (`@acme/ui`), through its `exports`
    /// map (`@acme/ui/server`), or by a deep path (`@acme/core/src/...`).
    Workspace,

    /// An npm package, e.g. `react` or `date-fns/locale`.
    External,

    /// A Node.js builtin, e.g. `node:fs/promises`, or `os` without the
    /// prefix.
    Builtin,
}

impl SyntheticRepo {
    /// Generate a repository with exactly `source_files` source files, in
    /// memory. About one package is made per 250 source files, with at least
    /// one library, one service and one app.
    ///
    /// # Panics
    ///
    /// If `source_files` is below [`MIN_SOURCE_FILES`].
    pub fn new(source_files: usize) -> Self {
        assert!(
            source_files >= MIN_SOURCE_FILES,
            "a synthetic repository needs at least {MIN_SOURCE_FILES} source files, not \
             {source_files}"
        );

        let mut rng = Rng::with_seed(SEED);
        let plans = plan::plan_packages(&mut rng, source_files);

        let mut files = Vec::new();
        let mut externals = BTreeSet::new();
        for plan in &plans {
            let (package_files, imported) = plan.generate(&mut rng, &plans);
            files.extend(package_files);
            externals.extend(imported);
        }
        files.extend(generator::root_files(&externals));
        files.sort_by(|a, b| a.path.cmp(&b.path));

        let mut packages: Vec<_> = plans.into_iter().map(|plan| plan.package).collect();
        packages.sort_by(|a, b| a.dir.cmp(&b.dir));

        Self { files, packages, externals: externals.into_iter().map(String::from).collect() }
    }

    /// Write every file to `root`, creating directories as needed.
    pub fn write(&self, root: &Path) -> io::Result<()> {
        let mut created = BTreeSet::new();
        for file in &self.files {
            let path = root.join(&file.path);
            if let Some(parent) = path.parent()
                && !created.contains(parent)
            {
                fs::create_dir_all(parent)?;
                created.insert(parent.to_path_buf());
            }
            fs::write(&path, &file.contents)?;
        }
        Ok(())
    }

    /// Every file, sorted by path.
    pub fn files(&self) -> &[SyntheticFile] {
        &self.files
    }

    /// The source files, which zenith's walk should find, sorted by path.
    pub fn source_files(&self) -> impl Iterator<Item = &SyntheticFile> {
        self.files.iter().filter(|file| file.kind == FileKind::Source)
    }

    /// Every import in the source files.
    pub fn imports(&self) -> impl Iterator<Item = &SyntheticImport> {
        self.files.iter().flat_map(|file| &file.imports)
    }

    /// The workspace packages, sorted by directory, as zenith's package
    /// discovery should find them.
    pub fn packages(&self) -> &[SyntheticPackage] {
        &self.packages
    }

    /// The names of the npm packages that the source files import, sorted.
    /// Each one has a stub in `node_modules`.
    pub fn externals(&self) -> &[String] {
        &self.externals
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use zenith_utils::path::posix_parent;

    use super::{plan::file_name, *};

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(SyntheticRepo::new(1_000), SyntheticRepo::new(1_000));
    }

    #[test]
    fn has_exactly_the_requested_source_files() {
        for size in [MIN_SOURCE_FILES, 333, 1_000, 4_321] {
            assert_eq!(SyntheticRepo::new(size).source_files().count(), size, "size {size}");
        }
    }

    #[test]
    fn paths_are_unique_and_sorted() {
        let repo = SyntheticRepo::new(2_000);
        let paths: Vec<&str> = repo.files().iter().map(|file| file.path.as_str()).collect();
        let unique: BTreeSet<&str> = paths.iter().copied().collect();
        assert_eq!(unique.len(), paths.len());
        assert!(paths.is_sorted());
    }

    #[test]
    fn packages_look_like_a_workspace() {
        let repo = SyntheticRepo::new(10_000);
        let packages = repo.packages();
        assert_eq!(packages.len(), 40);

        let names: BTreeSet<&str> = packages.iter().map(|package| package.name.as_str()).collect();
        assert_eq!(names.len(), packages.len(), "package names are unique");
        assert!(names.iter().all(|name| name.starts_with("@acme/")));
        for role in [PackageRole::Library, PackageRole::Service, PackageRole::App] {
            assert!(packages.iter().any(|package| package.role == role), "{role:?}");
        }
        assert!(packages.iter().any(|package| package.has_exports));
        assert!(packages.iter().any(|package| !package.has_exports));
        assert!(
            packages.iter().any(|package| !package.name.ends_with(file_name(&package.dir))),
            "some package names differ from their directories"
        );
        for package in packages {
            assert!(package.dependencies.iter().all(|name| names.contains(name.as_str())));
        }
    }

    /// Resolve a path to a file of the repository as a bundler would: `.js`
    /// stands for TypeScript, and a directory for its `index.ts`. Returns
    /// the path unchanged if nothing matches.
    fn resolve(files: &BTreeSet<&str>, path: &str) -> String {
        let mut segments: Vec<&str> = Vec::new();
        for segment in path.split('/') {
            match segment {
                "." => {}
                ".." => {
                    segments.pop();
                }
                segment => segments.push(segment),
            }
        }
        let path = segments.join("/");
        let stem = path.strip_suffix(".js").unwrap_or(&path);
        [path.clone(), format!("{stem}.ts"), format!("{stem}.tsx"), format!("{stem}/index.ts")]
            .into_iter()
            .find(|candidate| files.contains(candidate.as_str()))
            .unwrap_or(path)
    }

    /// Every specifier of a file in the repository leads to the file that its
    /// import says it resolves to, which keeps the generator honest for the
    /// benchmarks that resolve them.
    #[test]
    fn specifiers_resolve_to_their_files() {
        let repo = SyntheticRepo::new(10_000);
        let files: BTreeSet<&str> = repo.files().iter().map(|file| file.path.as_str()).collect();
        let packages = repo.packages();

        for file in repo.source_files() {
            let package =
                packages.iter().find(|p| file.path.starts_with(&format!("{}/", p.dir))).unwrap();
            for import in &file.imports {
                let specifier = &import.specifier;
                let resolved = match import.target {
                    ImportTarget::External | ImportTarget::Builtin => None,
                    ImportTarget::Relative => {
                        Some(resolve(&files, &format!("{}/{specifier}", posix_parent(&file.path))))
                    }
                    ImportTarget::Alias => {
                        let path = specifier.strip_prefix("@/").unwrap();
                        Some(resolve(&files, &format!("{}/src/{path}", package.dir)))
                    }
                    ImportTarget::Workspace => {
                        let dependency = packages
                            .iter()
                            .find(|p| {
                                *specifier == p.name
                                    || specifier.starts_with(&format!("{}/", p.name))
                            })
                            .unwrap();
                        assert!(package.dependencies.contains(&dependency.name), "{specifier}");
                        let dir = &dependency.dir;
                        Some(match specifier[dependency.name.len()..].trim_start_matches('/') {
                            "" => format!("{dir}/src/index.ts"),
                            "server" => format!("{dir}/src/server.ts"),
                            path if dependency.has_exports => {
                                format!("{dir}/src/{path}/index.ts")
                            }
                            path => resolve(&files, &format!("{dir}/{path}")),
                        })
                    }
                };
                assert_eq!(import.resolves_to, resolved, "`{specifier}` in {}", file.path);
                if let Some(resolved) = &resolved {
                    assert!(files.contains(resolved.as_str()), "{resolved} isn't generated");
                }
            }
        }
    }

    #[test]
    fn imports_have_a_realistic_mix() {
        let repo = SyntheticRepo::new(1_000);
        let imports: Vec<&SyntheticImport> = repo.imports().collect();

        let targets: BTreeSet<ImportTarget> = imports.iter().map(|import| import.target).collect();
        assert_eq!(targets.len(), 5, "every kind of target: {targets:?}");
        let kinds: BTreeSet<ImportKind> = imports.iter().map(|import| import.kind).collect();
        assert_eq!(kinds.len(), 4, "every kind of import: {kinds:?}");

        let per_file = imports.len() as f64 / 1_000.0;
        assert!((4.0..10.0).contains(&per_file), "{per_file} imports per source file");
        assert!(repo.externals().iter().any(|name| name == "react"));
    }

    #[test]
    fn has_files_for_the_walk_to_skip() {
        let repo = SyntheticRepo::new(1_000);
        let skipped: Vec<&str> = repo
            .files()
            .iter()
            .filter(|file| file.kind == FileKind::Skipped)
            .map(|file| file.path.as_str())
            .collect();
        for part in ["node_modules/", "/dist/", "/.next/", "/__generated__/"] {
            assert!(skipped.iter().any(|path| path.contains(part)), "nothing in `{part}`");
        }
    }
}
