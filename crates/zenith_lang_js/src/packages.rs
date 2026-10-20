//! Workspace package discovery: the packages that `package.json` `workspaces`
//! and `pnpm-workspace.yaml` declare.

use std::{
    collections::{HashMap, hash_map::Entry},
    io,
    path::{Path, PathBuf},
};

use log::warn;
use serde_json::Value;
use yaml_rust2::{Yaml, YamlLoader};
use zenith_lang::{Workspace, WorkspacePackage};
use zenith_utils::{
    fs::{ReadJsonError, read_json, read_text},
    glob::Glob,
    path::relative_posix,
};

/// Discover the workspace packages of the project at `root`, given the
/// `package.json` files found by the walk.
///
/// The packages are declared by the `workspaces` field of the root
/// `package.json` (an array, or `{ "packages": [...] }`) and by the `packages`
/// list of `pnpm-workspace.yaml`. Patterns starting with `!` exclude
/// directories. A matching directory is a package if its `package.json` has a
/// `name`.
pub fn discover_packages(root: &Path, package_jsons: &[PathBuf]) -> Workspace {
    let root_json = read_manifest(&root.join("package.json"));

    let mut patterns = root_json.as_ref().map(workspace_patterns).unwrap_or_default();
    patterns.extend(pnpm_patterns(&root.join("pnpm-workspace.yaml")));

    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    for pattern in &patterns {
        let (negated, pattern) = match pattern.strip_prefix('!') {
            Some(pattern) => (true, pattern),
            None => (false, pattern.as_str()),
        };
        let pattern = pattern.trim_start_matches("./").trim_matches('/');
        if pattern.is_empty() || pattern == "." {
            continue;
        }
        match Glob::new(pattern) {
            Ok(glob) if negated => excludes.push(glob),
            Ok(glob) => includes.push(glob),
            Err(error) => warn!("ignoring the invalid workspace pattern `{pattern}`: {error}"),
        }
    }

    let mut packages = Vec::new();
    for manifest in package_jsons {
        let Some(path) = manifest.parent() else {
            continue;
        };
        let Some(dir) = relative_posix(root, path) else {
            continue;
        };
        if dir.is_empty()
            || !includes.iter().any(|glob| glob.is_match(&dir))
            || excludes.iter().any(|glob| glob.is_match(&dir))
        {
            continue;
        }
        let Some(name) = read_manifest(manifest).as_ref().and_then(package_name) else {
            continue;
        };
        packages.push(WorkspacePackage { name, dir, path: path.to_path_buf() });
    }
    packages.sort_by(|a, b| a.dir.cmp(&b.dir));

    let root_package = root_json.as_ref().and_then(package_name).map(|name| WorkspacePackage {
        name,
        dir: String::new(),
        path: root.to_path_buf(),
    });

    let workspace = Workspace { packages, root_package };
    warn_about_duplicate_names(&workspace);
    workspace
}

/// Warn when two packages share a name, since a lookup by name finds only one
/// of them.
fn warn_about_duplicate_names(workspace: &Workspace) {
    let mut seen: HashMap<&str, &str> = HashMap::new();
    for package in workspace.packages.iter().chain(&workspace.root_package) {
        match seen.entry(&package.name) {
            Entry::Occupied(first) => warn!(
                "the packages in `{}` and `{}` are both named `{}`",
                display_dir(first.get()),
                display_dir(&package.dir),
                package.name
            ),
            Entry::Vacant(slot) => {
                slot.insert(&package.dir);
            }
        }
    }
}

/// A package directory for messages, with `.` for the root.
fn display_dir(dir: &str) -> &str {
    if dir.is_empty() { "." } else { dir }
}

/// Read a manifest, or `None` if it doesn't exist. Warns if it exists but
/// can't be read or parsed.
fn read_manifest(path: &Path) -> Option<Value> {
    match read_json(path) {
        Ok(value) => Some(value),
        Err(ReadJsonError::Read(error)) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            warn!("could not read `{}`: {error}", path.display());
            None
        }
    }
}

/// The non-empty `name` of a `package.json`.
fn package_name(json: &Value) -> Option<String> {
    json.get("name").and_then(Value::as_str).filter(|name| !name.is_empty()).map(String::from)
}

/// The `workspaces` field: either an array, or `{ "packages": [...] }`.
fn workspace_patterns(json: &Value) -> Vec<String> {
    let workspaces = match json.get("workspaces") {
        Some(Value::Object(object)) => object.get("packages"),
        other => other,
    };
    workspaces
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default()
}

/// The `packages` list of a `pnpm-workspace.yaml`.
fn pnpm_patterns(path: &Path) -> Vec<String> {
    let contents = match read_text(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            warn!("could not read `{}`: {error}", path.display());
            return Vec::new();
        }
    };
    let documents = match YamlLoader::load_from_str(&contents) {
        Ok(documents) => documents,
        Err(error) => {
            warn!("could not parse `{}`: {error}", path.display());
            return Vec::new();
        }
    };
    match documents.first().map(|document| &document["packages"]) {
        Some(Yaml::Array(items)) => {
            items.iter().filter_map(Yaml::as_str).map(String::from).collect()
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use zenith_utils::path::adjust_canonicalisation;

    use super::*;

    /// A temporary project with the given files, its canonical root, and the
    /// `package.json` files among them.
    fn project(files: &[(&str, &str)]) -> (TempDir, PathBuf, Vec<PathBuf>) {
        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        let mut manifests = Vec::new();
        for (file, contents) in files {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            if path.ends_with("package.json") {
                manifests.push(path);
            }
        }
        (dir, root, manifests)
    }

    fn found(workspace: &Workspace) -> Vec<(&str, &str)> {
        workspace.packages.iter().map(|p| (p.name.as_str(), p.dir.as_str())).collect()
    }

    #[test]
    fn discovers_packages_from_workspaces_and_pnpm() {
        let (_dir, root, manifests) = project(&[
            (
                "package.json",
                r#"{ "name": "acme", "workspaces": { "packages": ["packages/*", "web", "!packages/skip"] } }"#,
            ),
            ("packages/designSystem/package.json", r#"{ "name": "@acme/design-system" }"#),
            ("packages/skip/package.json", r#"{ "name": "skip" }"#),
            ("packages/nameless/package.json", r#"{ "private": true }"#),
            ("web/package.json", r#"{ "name": "web" }"#),
            ("tools/package.json", r#"{ "name": "tools" }"#),
            ("services/mcp/package.json", r#"{ "name": "@acme/mcp" }"#),
            ("pnpm-workspace.yaml", "packages:\n  - 'services/*'\n"),
        ]);

        let workspace = discover_packages(&root, &manifests);
        assert_eq!(
            found(&workspace),
            [
                ("@acme/design-system", "packages/designSystem"),
                ("@acme/mcp", "services/mcp"),
                ("web", "web"),
            ]
        );
        assert_eq!(workspace.root_package.as_ref().map(|p| p.name.as_str()), Some("acme"));
        assert_eq!(workspace.package("acme").map(|p| p.dir.as_str()), Some(""));
        assert_eq!(workspace.package("web").map(|p| p.path.clone()), Some(root.join("web")));
        assert_eq!(workspace.package("tools"), None);
    }

    #[test]
    fn accepts_array_workspaces_and_loose_patterns() {
        let (_dir, root, manifests) = project(&[
            ("package.json", r#"{ "workspaces": ["./apps/*/", "/tools", ".", "apps/{a"] }"#),
            ("apps/site/package.json", r#"{ "name": "site" }"#),
            ("tools/package.json", r#"{ "name": "tools" }"#),
        ]);

        let workspace = discover_packages(&root, &manifests);
        assert_eq!(found(&workspace), [("site", "apps/site"), ("tools", "tools")]);
        assert_eq!(workspace.root_package, None);
    }

    #[test]
    fn pnpm_patterns_nest_and_negate() {
        let (_dir, root, manifests) = project(&[
            ("package.json", r#"{ "name": "root" }"#),
            ("packages/a/package.json", r#"{ "name": "a" }"#),
            ("packages/group/b/package.json", r#"{ "name": "b" }"#),
            ("packages/group/test/package.json", r#"{ "name": "test" }"#),
            ("pnpm-workspace.yaml", "packages:\n  - 'packages/**'\n  - '!**/test'\n"),
        ]);

        let workspace = discover_packages(&root, &manifests);
        assert_eq!(found(&workspace), [("a", "packages/a"), ("b", "packages/group/b")]);
    }

    #[test]
    fn reads_manifests_with_a_byte_order_mark() {
        let (_dir, root, manifests) = project(&[
            ("package.json", "\u{feff}{ \"name\": \"root\", \"workspaces\": [\"web\"] }"),
            ("web/package.json", r#"{ "name": "web" }"#),
            ("services/api/package.json", r#"{ "name": "api" }"#),
            ("pnpm-workspace.yaml", "\u{feff}packages:\n  - 'services/*'\n"),
        ]);

        let workspace = discover_packages(&root, &manifests);
        assert_eq!(found(&workspace), [("api", "services/api"), ("web", "web")]);
        assert_eq!(workspace.root_package.map(|p| p.name), Some("root".to_string()));
    }

    #[test]
    fn tolerates_unparsable_manifests() {
        let (_dir, root, manifests) = project(&[
            ("package.json", r#"{ "workspaces": ["apps/*"] }"#),
            ("apps/broken/package.json", "{ not json"),
            ("apps/site/package.json", r#"{ "name": "site" }"#),
            ("pnpm-workspace.yaml", "packages: [unclosed"),
        ]);

        let workspace = discover_packages(&root, &manifests);
        assert_eq!(found(&workspace), [("site", "apps/site")]);
    }

    #[test]
    fn without_a_root_manifest_there_are_no_packages() {
        let (_dir, root, manifests) = project(&[("web/package.json", r#"{ "name": "web" }"#)]);

        let workspace = discover_packages(&root, &manifests);
        assert!(workspace.packages.is_empty());
        assert_eq!(workspace.root_package, None);
    }
}
