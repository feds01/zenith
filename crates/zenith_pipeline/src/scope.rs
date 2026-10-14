//! The part of a project that a command reports on.

/// The files that a command reports on: those under some paths of the
/// project, or every file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    /// The paths, as POSIX paths relative to the root, or empty for every
    /// file. The root itself is the empty path.
    paths: Vec<String>,
}

impl Scope {
    /// The files under `paths`, POSIX paths relative to the root. No paths
    /// is every file.
    pub fn new(paths: Vec<String>) -> Self {
        Self { paths }
    }

    /// Whether the scope is every file.
    pub fn is_everything(&self) -> bool {
        self.paths.is_empty() || self.paths.iter().any(String::is_empty)
    }

    /// Whether the node `name` is in the scope: one of its paths, or under
    /// one of them.
    pub fn contains(&self, name: &str) -> bool {
        self.is_everything() || self.paths.iter().any(|path| is_under(name, path))
    }

    /// Whether the node `name` is in the scope, or has one of its paths under
    /// it, as a directory does.
    pub fn overlaps(&self, name: &str) -> bool {
        self.contains(name) || self.paths.iter().any(|path| is_under(path, name))
    }
}

/// Whether `name` is `path`, or under it.
fn is_under(name: &str, path: &str) -> bool {
    name.strip_prefix(path).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_the_files_under_its_paths() {
        let scope = Scope::new(vec!["web/src".into(), "api/main.ts".into()]);
        assert!(scope.contains("web/src"));
        assert!(scope.contains("web/src/a.ts"));
        assert!(scope.contains("api/main.ts"));
        assert!(!scope.contains("web/srcs/a.ts"));
        assert!(!scope.contains("web"));
        assert!(!scope.contains("npm:react"));
        assert!(!scope.is_everything());
    }

    #[test]
    fn overlaps_the_directories_above_its_paths() {
        let scope = Scope::new(vec!["web/src/a.ts".into()]);
        assert!(scope.overlaps("web"));
        assert!(scope.overlaps("web/src"));
        assert!(!scope.overlaps("web/s"));
        assert!(!scope.overlaps("api"));
    }

    #[test]
    fn no_paths_or_the_root_is_everything() {
        for scope in [Scope::default(), Scope::new(vec!["web".into(), String::new()])] {
            assert!(scope.is_everything());
            assert!(scope.contains("api/main.ts") && scope.contains("npm:react"));
        }
    }
}
