//! The packages of a project, as every language finds them.

use std::path::PathBuf;

use zenith_utils::glob::NamePattern;

/// A package of the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePackage {
    /// The `name` from its `package.json`.
    pub name: String,

    /// Its directory, as a POSIX path relative to the root (`""` for the
    /// root package).
    pub dir: String,

    /// Its directory, as an absolute path.
    pub path: PathBuf,
}

/// The packages of a project.
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    /// The workspace packages, sorted by directory. Never includes the root
    /// package.
    pub packages: Vec<WorkspacePackage>,

    /// The root package, if the root has a named `package.json`.
    pub root_package: Option<WorkspacePackage>,
}

impl Workspace {
    /// Add the packages of `other`, another language's, keeping the packages
    /// sorted by directory. The first root package found stays.
    pub fn merge(&mut self, other: Workspace) {
        self.packages.extend(other.packages);
        self.packages.sort_by(|a, b| a.dir.cmp(&b.dir));
        self.root_package = self.root_package.take().or(other.root_package);
    }

    /// Look up a package by name, including the root package.
    pub fn package(&self, name: &str) -> Option<&WorkspacePackage> {
        self.packages.iter().chain(&self.root_package).find(|package| package.name == name)
    }

    /// The packages whose name matches `pattern`, in order of their
    /// directories, as `pkg:` selectors pick them. The root package contains
    /// every other, so only its exact name matches it, e.g. `pkg:*` never
    /// does, and it comes last.
    pub fn find_packages<'w, 'p>(
        &'w self,
        pattern: &'p NamePattern,
    ) -> impl Iterator<Item = &'w WorkspacePackage> + use<'w, 'p> {
        let root = self
            .root_package
            .as_ref()
            .filter(|root| pattern.is_literal() && pattern.is_match(&root.name));
        self.packages.iter().filter(|package| pattern.is_match(&package.name)).chain(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_packages_by_name_but_the_root_only_by_its_exact_name() {
        let package = |name: &str, dir: &str| WorkspacePackage {
            name: name.to_string(),
            dir: dir.to_string(),
            path: PathBuf::from("/repo").join(dir),
        };
        let workspace = Workspace {
            packages: vec![package("@acme/api", "packages/api"), package("web", "web")],
            root_package: Some(package("acme", "")),
        };
        let find = |pattern: &str| -> Vec<&str> {
            let pattern = NamePattern::new(pattern).unwrap();
            workspace.find_packages(&pattern).map(|package| package.name.as_str()).collect()
        };
        assert_eq!(find("*"), ["@acme/api", "web"]);
        assert_eq!(find("@acme/*"), ["@acme/api"]);
        assert_eq!(find("acme"), ["acme"]);
        assert!(find("acm?").is_empty());
        assert!(find("missing").is_empty());
    }
}
