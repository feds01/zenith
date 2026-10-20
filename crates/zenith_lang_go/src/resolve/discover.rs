//! Building a [`GoModules`] from the `go.mod` and `go.work` files that the
//! walker finds, whatever reads their text.

use std::{error::Error, fmt, io};

use zenith_utils::path::posix_parent;

use super::{GoModule, GoModules, GoWorkspace};
use crate::modules::{GoMod, GoWork, ModError};

/// Why a `go.mod` or `go.work` file did not make it into the set whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoverErrorKind {
    /// The file could not be read. It is left out of the set.
    Read(String),

    /// A line of the file is malformed. The rest of the file is kept.
    Parse(ModError),
}

/// A problem with one `go.mod` or `go.work` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoverError {
    /// The file, as a POSIX path relative to the project root.
    pub file: String,

    /// What is wrong with it.
    pub kind: DiscoverErrorKind,
}

impl fmt::Display for DiscoverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            DiscoverErrorKind::Read(message) => write!(f, "{}: cannot read: {message}", self.file),
            DiscoverErrorKind::Parse(error) => write!(f, "{}: {error}", self.file),
        }
    }
}

impl Error for DiscoverError {}

/// The modules found in a project, and the files that were trouble.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovered {
    /// The modules and workspaces that were read.
    pub modules: GoModules,

    /// The files that could not be read or parsed, in the order of the files
    /// that were given.
    pub errors: Vec<DiscoverError>,
}

impl GoModules {
    /// Read the `go.mod` and `go.work` files among `files`, which are POSIX
    /// paths relative to the project root, with `read` giving the text of one.
    /// Any other file is skipped.
    ///
    /// A file that `read` fails on is left out and reported; one that does not
    /// parse is kept for what could be read, and reported.
    pub fn discover<I, R>(files: I, mut read: R) -> Discovered
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
        R: FnMut(&str) -> io::Result<String>,
    {
        let mut modules = Vec::new();
        let mut workspaces = Vec::new();
        let mut errors = Vec::new();

        for file in files {
            let file = file.as_ref();
            let (dir, name) = (posix_parent(file), file.rsplit('/').next().unwrap_or(file));
            if !matches!(name, "go.mod" | "go.work") {
                continue;
            }
            let source = match read(file) {
                Ok(source) => source,
                Err(error) => {
                    errors.push(DiscoverError {
                        file: file.to_owned(),
                        kind: DiscoverErrorKind::Read(error.to_string()),
                    });
                    continue;
                }
            };
            let dir = dir.to_owned();
            let parse_errors = if name == "go.mod" {
                let parsed = GoMod::parse(&source);
                modules.push(GoModule { dir, go_mod: parsed.value });
                parsed.errors
            } else {
                let parsed = GoWork::parse(&source);
                workspaces.push(GoWorkspace { dir, work: parsed.value });
                parsed.errors
            };
            errors.extend(parse_errors.into_iter().map(|error| DiscoverError {
                file: file.to_owned(),
                kind: DiscoverErrorKind::Parse(error),
            }));
        }

        Discovered { modules: GoModules::new(modules, workspaces), errors }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::resolve::GoResolved;

    fn reader<'a>(files: &'a HashMap<&str, &str>) -> impl FnMut(&str) -> io::Result<String> + 'a {
        |path| {
            files.get(path).map(|text| (*text).to_owned()).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("no such file {path}"))
            })
        }
    }

    #[test]
    fn modules_and_workspaces_are_read_from_their_paths() {
        let files = HashMap::from([
            ("go.work", "use ./api\nuse ./shared\n"),
            ("api/go.mod", "module example.com/api\nrequire github.com/x/y v1.0.0\n"),
            ("shared/go.mod", "module example.com/shared\n"),
        ]);
        let paths = ["shared/go.mod", "go.work", "api/go.mod", "api/main.go", "README.md"];
        let found = GoModules::discover(paths, reader(&files));
        assert_eq!(found.errors, []);
        let dirs: Vec<_> = found.modules.modules().map(|m| m.dir.as_str()).collect();
        assert_eq!(dirs, ["api", "shared"]);
        assert_eq!(
            found.modules.resolve("api", "example.com/shared/log"),
            GoResolved::Internal { dir: "shared/log".to_owned() }
        );
        assert_eq!(
            found.modules.resolve("api", "github.com/x/y"),
            GoResolved::External { module: "github.com/x/y".to_owned(), required: true }
        );
    }

    #[test]
    fn a_root_go_mod_is_in_the_root_directory() {
        let files = HashMap::from([("go.mod", "module myapp\n")]);
        let found = GoModules::discover(vec!["go.mod".to_owned()], reader(&files));
        assert_eq!(found.modules.module_of("any/dir").map(|m| m.dir.as_str()), Some(""));
        assert_eq!(
            found.modules.resolve("cmd", "myapp/util"),
            GoResolved::Internal { dir: "util".to_owned() }
        );
    }

    #[test]
    fn only_files_named_go_mod_or_go_work_count() {
        let files = HashMap::from([("a/go.mod", "module a\n")]);
        let mut asked = Vec::new();
        let found = GoModules::discover(
            ["a/go.mod", "a/go.sum", "a/go.work.sum", "a/ngo.mod", "a/go.mod.bak", "go.mod/x.go"],
            |path| {
                asked.push(path.to_owned());
                reader(&files)(path)
            },
        );
        assert_eq!(asked, ["a/go.mod"]);
        assert_eq!(found.modules.modules().count(), 1);
    }

    #[test]
    fn an_unreadable_file_is_reported_and_left_out() {
        let files = HashMap::from([("b/go.mod", "module b\n")]);
        let found = GoModules::discover(["a/go.mod", "b/go.mod"], reader(&files));
        assert_eq!(found.modules.modules().count(), 1);
        assert_eq!(found.errors.len(), 1);
        assert_eq!(found.errors[0].file, "a/go.mod");
        assert!(matches!(found.errors[0].kind, DiscoverErrorKind::Read(_)));
        assert!(found.errors[0].to_string().starts_with("a/go.mod: cannot read: "));
    }

    #[test]
    fn a_malformed_file_is_reported_and_kept_for_what_it_holds() {
        let files = HashMap::from([
            ("a/go.mod", "module example.com/a\nrequire broken\nrequire ok v1\n"),
            ("go.work", "use\nuse ./a\n"),
        ]);
        let found = GoModules::discover(["a/go.mod", "go.work"], reader(&files));
        let reported: Vec<_> = found
            .errors
            .iter()
            .map(|error| match &error.kind {
                DiscoverErrorKind::Parse(parse) => (error.file.as_str(), parse.line),
                DiscoverErrorKind::Read(_) => unreachable!(),
            })
            .collect();
        assert_eq!(reported, [("a/go.mod", 2), ("go.work", 1)]);
        assert_eq!(
            found.errors[0].to_string(),
            "a/go.mod: line 2: `require` takes 2 operands, found 1"
        );
        assert_eq!(found.modules.modules().next().unwrap().go_mod.requires.len(), 1);
        assert_eq!(
            found.modules.resolve("", "example.com/a/x"),
            GoResolved::Internal { dir: "a/x".to_owned() }
        );
    }

    #[test]
    fn nothing_found_is_an_empty_set() {
        let files = HashMap::new();
        let found = GoModules::discover(Vec::<String>::new(), reader(&files));
        assert!(found.modules.is_empty());
        assert_eq!(found.errors, []);
    }
}
