//! Finding the files that the root's configuration file is split over: the
//! files it includes, and its members' files.
//!
//! Every glob is matched against the files of the project that the caller's
//! walk finds, walked once, when a glob is first matched. The walk skips what
//! zenith never scans, so a glob doesn't match what `[files] exclude`
//! excludes or `.gitignore` ignores, with or without a `**` segment.

use std::{
    cell::LazyCell,
    path::{Path, PathBuf},
};

use zenith_utils::{fs::FileSystem, glob::Glob, location::ByteRange, path::relative_posix};

use crate::error::{ConfigError, ConfigErrorKind};

/// The name of the configuration file, of the root and of each member.
pub const CONFIG_FILE_NAME: &str = "zenith.toml";

/// The globs of the files that a configuration file is split over, relative
/// to the root: the files it includes, and, in the root's, its members.
#[derive(Debug, Default)]
pub(crate) struct PartGlobs {
    /// The globs of the files it includes.
    pub(crate) include: Vec<PartGlob>,

    /// The globs of its members' directories.
    pub(crate) members: Vec<PartGlob>,
}

/// A glob of `include` or `[workspace] members`.
#[derive(Debug)]
pub(crate) struct PartGlob {
    /// The glob as written.
    pub(crate) pattern: String,

    /// The glob, relative to the root.
    pub(crate) glob: Glob,

    /// Where it is written in its file.
    pub(crate) range: ByteRange,
}

/// A configuration file that is part of the root's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Part {
    /// Its path.
    pub(crate) path: PathBuf,

    /// The directory of the member whose file it is, or that includes it, as
    /// a POSIX path relative to the root. `None` for the root's.
    pub(crate) member: Option<String>,
}

/// The files of a project, and the directories that hold them, as POSIX
/// paths relative to its root, in order.
struct ProjectFiles {
    /// The files.
    files: Vec<String>,

    /// The directories that hold them.
    dirs: Vec<String>,
}

impl ProjectFiles {
    /// The files at `paths`, under `root`, and the directories that hold
    /// them.
    fn new(root: &Path, paths: Vec<PathBuf>) -> Self {
        let mut files: Vec<_> =
            paths.iter().filter_map(|path| relative_posix(root, path)).collect();
        files.sort();
        let mut dirs: Vec<_> = (files.iter())
            .flat_map(|file| file.match_indices('/').map(|(end, _)| file[..end].to_string()))
            .collect();
        dirs.sort();
        dirs.dedup();
        Self { files, dirs }
    }
}

/// A configuration file that a [`PartSearch`] found.
struct Found {
    /// Its canonical path.
    path: PathBuf,

    /// The directory of the member that it is part of, or `None` for the
    /// root's.
    member: Option<String>,

    /// Whether it is the member's own file, or the root's, rather than one
    /// that is included.
    own: bool,

    /// Where the glob that matched it is written, in the file that has it.
    range: ByteRange,
}

/// The walk of the project that a [`PartSearch`] matches its globs against.
type Walk<'a> = Box<dyn FnOnce() -> ProjectFiles + 'a>;

/// The search for the files that the root's configuration file is split
/// over, which finds each file once.
pub(crate) struct PartSearch<'a> {
    /// The file system of the project.
    file_system: &'a dyn FileSystem,

    /// The project root.
    root: &'a Path,

    /// The path of the root's file, relative to the root, for errors.
    root_file: String,

    /// The files of the project, walked when a glob is first matched.
    project: LazyCell<ProjectFiles, Walk<'a>>,

    /// The root's file, and the files found so far.
    found: Vec<Found>,
}

impl<'a> PartSearch<'a> {
    /// A search under `root`, in the file system, for the files that the
    /// root's, at `config`, is split over, among those that `walk` finds. It
    /// is called once, when a glob is first matched.
    pub(crate) fn new(
        file_system: &'a dyn FileSystem,
        root: &'a Path,
        config: &Path,
        walk: impl FnOnce() -> Vec<PathBuf> + 'a,
    ) -> Self {
        let root_file =
            relative_posix(root, config).unwrap_or_else(|| config.display().to_string());
        let walk: Walk<'a> = Box::new(move || ProjectFiles::new(root, walk()));
        let path = file_system.adjust_canonicalisation(config);
        let found = vec![Found { path, member: None, own: true, range: ByteRange::default() }];
        Self { file_system, root, root_file, project: LazyCell::new(walk), found }
    }

    /// The files that `globs`, of the file of the `member` in that directory
    /// or of the root's, match and that haven't been found yet, leaving out
    /// what the `exclude` globs of that file, and of those that include it,
    /// exclude: the files it includes, which are part of the same member,
    /// then the `zenith.toml` of each member's directory that has one, each
    /// glob's in the order of their paths. Errors are added to `errors`: a
    /// glob that matches nothing, a member's own file that is included, and a
    /// file that is included as part of two members, or of a member and of
    /// the root's.
    pub(crate) fn find(
        &mut self,
        globs: &PartGlobs,
        member: Option<&str>,
        exclude: &[Glob],
        errors: &mut Vec<ConfigError>,
    ) -> Vec<Part> {
        let mut parts = Vec::new();
        for (globs, are_members) in [(&globs.include, false), (&globs.members, true)] {
            for PartGlob { pattern, glob, range } in globs {
                let found = match are_members {
                    true => self.members(glob, exclude),
                    false => self.files(glob, exclude, member),
                };
                let Some(found) = found else {
                    let kind = ConfigErrorKind::UnmatchedPattern { pattern: pattern.clone() };
                    errors.push(ConfigError { kind, contract: None, range: Some(*range) });
                    continue;
                };
                for part in found {
                    let path = self.file_system.adjust_canonicalisation(&part.path);
                    let Some(first) = self.found.iter().find(|found| found.path == path) else {
                        let (member, own, range) = (part.member.clone(), are_members, *range);
                        self.found.push(Found { path, member, own, range });
                        parts.push(part);
                        continue;
                    };
                    if first.member == part.member {
                        continue;
                    }
                    let path = relative_posix(self.root, &part.path).unwrap_or_default();
                    // A member's own file is found after the root's includes, which are written
                    // in the same file, so the include is the one to report.
                    let (kind, range) = match (are_members, first.own, &first.member) {
                        (true, _, _) => {
                            let member = part.member.clone().unwrap_or_default();
                            (ConfigErrorKind::IncludesMember { path, member }, first.range)
                        }
                        (false, true, Some(member)) => {
                            let member = member.clone();
                            (ConfigErrorKind::IncludesMember { path, member }, *range)
                        }
                        (false, _, first) => {
                            let first = match first {
                                Some(member) => format!("{member}/{CONFIG_FILE_NAME}"),
                                None => self.root_file.clone(),
                            };
                            (ConfigErrorKind::PartOfTwo { path, first }, *range)
                        }
                    };
                    errors.push(ConfigError { kind, contract: None, range: Some(range) });
                }
            }
        }
        parts
    }

    /// The files that `glob` matches, and the `exclude` globs don't, which
    /// are part of the `member`, or `None` if it matches nothing.
    fn files(&self, glob: &Glob, exclude: &[Glob], member: Option<&str>) -> Option<Vec<Part>> {
        let parts: Vec<_> = (self.project.files.iter())
            .filter(|file| glob.is_match(file) && !is_excluded(file, exclude))
            .map(|file| Part { path: self.root.join(file), member: member.map(String::from) })
            .collect();
        Some(parts).filter(|parts| !parts.is_empty())
    }

    /// The `zenith.toml` of each member's directory that `glob` matches, and
    /// the `exclude` globs don't, that has one, or `None` if it matches no
    /// directory.
    fn members(&self, glob: &Glob, exclude: &[Glob]) -> Option<Vec<Part>> {
        let ProjectFiles { files, dirs } = &*self.project;
        let dirs: Vec<_> =
            dirs.iter().filter(|dir| glob.is_match(dir) && !is_excluded(dir, exclude)).collect();
        if dirs.is_empty() {
            return None;
        }
        let part = |dir: &String| {
            let file = format!("{dir}/{CONFIG_FILE_NAME}");
            files
                .binary_search(&file)
                .is_ok()
                .then(|| Part { path: self.root.join(file), member: Some(dir.clone()) })
        };
        Some(dirs.into_iter().filter_map(part).collect())
    }
}

/// Whether one of the `exclude` globs matches `path`, a POSIX path relative
/// to the root, or a directory that it is in.
fn is_excluded(path: &str, exclude: &[Glob]) -> bool {
    let dirs = path.match_indices('/').map(|(end, _)| &path[..end]);
    dirs.chain([path]).any(|path| exclude.iter().any(|glob| glob.is_match(path)))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use zenith_utils::fs::OsFileSystem;

    use super::*;
    use crate::testing::walk_files;

    /// A project with the files at `paths`, each empty, and its canonical
    /// root, which the walk needs.
    fn project(paths: &[&str]) -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        for path in paths {
            let path = dir.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        let root = dir.path().canonicalize().unwrap();
        (dir, root)
    }

    fn globs(patterns: &[&str]) -> Vec<PartGlob> {
        let glob = |pattern: &&str| PartGlob {
            pattern: pattern.to_string(),
            glob: Glob::new(pattern).unwrap(),
            range: ByteRange::default(),
        };
        patterns.iter().map(glob).collect()
    }

    /// A search of the project at `root`, whose walk skips `excluded/`.
    fn search(root: &Path) -> PartSearch<'_> {
        let walk = || walk_files(&OsFileSystem, root, &[Glob::new("excluded").unwrap()]);
        PartSearch::new(&OsFileSystem, root, &root.join(CONFIG_FILE_NAME), walk)
    }

    fn included(root: &Path, path: &str, member: Option<&str>) -> Part {
        Part { path: root.join(path), member: member.map(String::from) }
    }

    fn member(root: &Path, dir: &str) -> Part {
        Part { path: root.join(dir).join(CONFIG_FILE_NAME), member: Some(dir.to_string()) }
    }

    fn messages(errors: &[ConfigError]) -> Vec<String> {
        errors.iter().map(|error| error.kind.to_string()).collect()
    }

    #[test]
    fn finds_included_files_then_members_with_a_config_file() {
        let (_dir, root) = project(&[
            "zenith.toml",
            "zenith/b.toml",
            "zenith/a.toml",
            "zenith/notes.md",
            "apps/web/zenith.toml",
            "apps/admin/src/main.ts",
            "services/core/api/zenith.toml",
        ]);
        let globs = PartGlobs {
            include: globs(&["zenith/*.toml", "zenith.toml"]),
            members: globs(&["apps/*", "services/*/*"]),
        };
        let mut errors = Vec::new();
        let parts = search(&root).find(&globs, None, &[], &mut errors);
        assert_eq!(
            parts,
            [
                included(&root, "zenith/a.toml", None),
                included(&root, "zenith/b.toml", None),
                member(&root, "apps/web"),
                member(&root, "services/core/api"),
            ]
        );
        // The root's own file matches, so the glob isn't unmatched.
        assert_eq!(errors, []);
    }

    #[test]
    fn globs_with_or_without_a_double_star_skip_what_the_walk_skips() {
        let (_dir, root) = project(&[
            "zenith.toml",
            "rules/a.toml",
            "rules/deep/b.toml",
            "services/zenith.toml",
            "services/core/api/zenith.toml",
            "services/billing/zenith.toml",
            "excluded/c.toml",
            "excluded/rules/d.toml",
            "excluded/api/zenith.toml",
        ]);
        let globs = PartGlobs {
            include: globs(&["rules/**/*.toml", "excluded/*.toml", "excluded/**/*.toml"]),
            members: globs(&["services/**", "excluded/*"]),
        };
        let mut errors = Vec::new();
        let parts = search(&root).find(&globs, None, &[], &mut errors);
        // `services/**` is strictly inside `services`.
        assert_eq!(
            parts,
            [
                included(&root, "rules/a.toml", None),
                included(&root, "rules/deep/b.toml", None),
                member(&root, "services/billing"),
                member(&root, "services/core/api"),
            ]
        );
        assert_eq!(
            messages(&errors),
            [
                "`excluded/*.toml` matches nothing",
                "`excluded/**/*.toml` matches nothing",
                "`excluded/*` matches nothing",
            ]
        );
    }

    #[test]
    fn a_member_s_included_files_are_part_of_it_and_each_file_is_found_once() {
        let (_dir, root) =
            project(&["zenith.toml", "apps/web/zenith.toml", "apps/web/zenith/a.toml"]);
        let mut search = search(&root);
        let mut errors = Vec::new();
        let parts = search.find(
            &PartGlobs { members: globs(&["apps/*"]), ..Default::default() },
            None,
            &[],
            &mut errors,
        );
        assert_eq!(parts, [member(&root, "apps/web")]);
        let globs = PartGlobs {
            include: globs(&["apps/web/zenith/*.toml", "apps/web/zenith.toml"]),
            ..Default::default()
        };
        let parts = search.find(&globs, Some("apps/web"), &[], &mut errors);
        assert_eq!(parts, [included(&root, "apps/web/zenith/a.toml", Some("apps/web"))]);
        assert_eq!(search.find(&globs, Some("apps/web"), &[], &mut errors), []);
        assert_eq!(errors, []);
    }

    #[test]
    fn a_member_s_includes_skip_what_it_excludes() {
        let (_dir, root) = project(&[
            "zenith.toml",
            "apps/web/zenith.toml",
            "apps/web/rules/a.toml",
            "apps/web/generated/rules/b.toml",
        ]);
        let globs =
            PartGlobs { include: globs(&["apps/web/**/rules/*.toml"]), ..Default::default() };
        let exclude = [Glob::new("apps/web/generated").unwrap()];
        let mut errors = Vec::new();
        let parts = search(&root).find(&globs, Some("apps/web"), &exclude, &mut errors);
        assert_eq!(parts, [included(&root, "apps/web/rules/a.toml", Some("apps/web"))]);
        assert_eq!(errors, []);
    }

    #[test]
    fn a_file_can_only_be_part_of_one_member_or_of_the_root_s() {
        let (_dir, root) = project(&[
            "zenith.toml",
            "apps/web/zenith.toml",
            "apps/web/shared.toml",
            "apps/web/admin/zenith.toml",
        ]);
        let mut errors = Vec::new();

        // 1. The root includes a member's file.
        let root_s =
            PartGlobs { include: globs(&["apps/web/zenith.toml"]), members: globs(&["apps/*"]) };
        let parts = search(&root).find(&root_s, None, &[], &mut errors);
        assert_eq!(parts, [included(&root, "apps/web/zenith.toml", None)]);

        // 2. A member includes a file that the root does, and another member's file.
        let mut search = search(&root);
        let root_s =
            PartGlobs { include: globs(&["apps/web/shared.toml"]), members: globs(&["apps/**"]) };
        let parts = search.find(&root_s, None, &[], &mut errors);
        assert_eq!(
            parts,
            [
                included(&root, "apps/web/shared.toml", None),
                member(&root, "apps/web"),
                member(&root, "apps/web/admin"),
            ]
        );
        let web_s = PartGlobs { include: globs(&["apps/web/**/*.toml"]), ..Default::default() };
        assert_eq!(search.find(&web_s, Some("apps/web"), &[], &mut errors), []);
        assert_eq!(
            messages(&errors),
            [
                "`apps/web/zenith.toml` is the configuration file of the member `apps/web`, so it \
                 can't be included",
                "`apps/web/admin/zenith.toml` is the configuration file of the member \
                 `apps/web/admin`, so it can't be included",
                "`apps/web/shared.toml` is already part of the configuration in `zenith.toml`",
            ]
        );
    }

    #[test]
    fn a_glob_that_matches_nothing_is_an_error() {
        let (_dir, root) = project(&["zenith.toml", "apps/web/zenith.toml"]);
        let globs = PartGlobs {
            include: globs(&["zenith/*.toml", "zenith/**/*.toml"]),
            members: globs(&["app/*", "app/**"]),
        };
        let mut errors = Vec::new();
        assert_eq!(search(&root).find(&globs, None, &[], &mut errors), []);
        assert_eq!(
            messages(&errors),
            [
                "`zenith/*.toml` matches nothing",
                "`zenith/**/*.toml` matches nothing",
                "`app/*` matches nothing",
                "`app/**` matches nothing",
            ]
        );
    }
}
