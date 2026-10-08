//! Workspace package discovery.

use std::{
    fs,
    path::{Path, PathBuf},
};

use log::warn;
use serde_json::Value;
use yaml_rust2::{Yaml, YamlLoader};
use zenith_utils::{glob::Glob, path::relative_posix};

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
    /// Look up a package by name, including the root package.
    pub fn package(&self, name: &str) -> Option<&WorkspacePackage> {
        self.packages.iter().chain(&self.root_package).find(|package| package.name == name)
    }
}

/// Discover the workspace packages of the project at `root`, given the
/// `package.json` files found by the walk.
pub fn discover_packages(root: &Path, package_jsons: &[PathBuf]) -> Workspace {
    let root_manifest = root.join("package.json");
    let root_json = read_json(&root_manifest);

    let mut patterns = root_json.as_ref().map(workspace_patterns).unwrap_or_default();
    patterns.extend(pnpm_patterns(&root.join("pnpm-workspace.yaml")));

    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    for pattern in patterns {
        let (negated, pattern) = match pattern.strip_prefix('!') {
            Some(pattern) => (true, pattern),
            None => (false, pattern.as_str()),
        };
        let pattern = pattern.trim_start_matches("./").trim_end_matches('/');
        if pattern.is_empty() || pattern == "." {
            continue;
        }
        match Glob::new(pattern) {
            Ok(glob) if negated => excludes.push(glob),
            Ok(glob) => includes.push(glob),
            Err(error) => warn!("ignoring invalid workspace pattern `{pattern}`: {error}"),
        }
    }

    let mut packages = Vec::new();
    for manifest in package_jsons {
        let Some(dir_path) = manifest.parent() else { continue };
        let Some(dir) = relative_posix(root, dir_path) else { continue };
        if dir.is_empty() {
            continue;
        }
        if !includes.iter().any(|glob| glob.is_match(&dir))
            || excludes.iter().any(|glob| glob.is_match(&dir))
        {
            continue;
        }
        let Some(name) = read_json(manifest).as_ref().and_then(package_name) else {
            continue;
        };
        packages.push(WorkspacePackage { name, dir, path: dir_path.to_path_buf() });
    }
    packages.sort_by(|a, b| a.dir.cmp(&b.dir));

    let root_package = root_json.as_ref().and_then(package_name).map(|name| WorkspacePackage {
        name,
        dir: String::new(),
        path: root.to_path_buf(),
    });

    Workspace { packages, root_package }
}

fn read_json(path: &Path) -> Option<Value> {
    let contents = fs::read_to_string(path).ok()?;
    match serde_json::from_str(&contents) {
        Ok(value) => Some(value),
        Err(error) => {
            warn!("could not parse `{}`: {error}", path.display());
            None
        }
    }
}

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
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
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
    use super::*;

    #[test]
    fn discovers_packages_from_workspaces_and_pnpm() {
        let dir = std::env::temp_dir().join(format!("zenith-packages-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let root = zenith_utils::path::adjust_canonicalisation(&dir);
        let write = |path: &str, contents: &str| {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            path
        };
        let manifests = vec![
            write(
                "package.json",
                r#"{ "name": "acme", "workspaces": { "packages": ["common/*", "web", "!common/skip"] } }"#,
            ),
            write("common/browserEditor/package.json", r#"{ "name": "browser-editor" }"#),
            write("common/skip/package.json", r#"{ "name": "skip" }"#),
            write("common/nameless/package.json", r#"{ "private": true }"#),
            write("web/package.json", r#"{ "name": "web" }"#),
            write("tools/package.json", r#"{ "name": "tools" }"#),
            write("services/mcp/package.json", r#"{ "name": "@acme/mcp" }"#),
        ];
        write("pnpm-workspace.yaml", "packages:\n  - 'services/*'\n");

        let workspace = discover_packages(&root, &manifests);
        let found: Vec<_> =
            workspace.packages.iter().map(|p| (p.name.as_str(), p.dir.as_str())).collect();
        assert_eq!(
            found,
            vec![
                ("browser-editor", "common/browserEditor"),
                ("@acme/mcp", "services/mcp"),
                ("web", "web"),
            ]
        );
        assert_eq!(workspace.root_package.as_ref().map(|p| p.name.as_str()), Some("acme"));
        assert_eq!(workspace.package("acme").map(|p| p.dir.as_str()), Some(""));

        let _ = fs::remove_dir_all(&dir);
    }
}
