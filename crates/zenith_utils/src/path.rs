//! Path manipulation and canonicalisation utilities.
//!
//! Zenith identifies every internal node by a POSIX path relative to the
//! project root, on every platform.

use std::path::{Component, Path, PathBuf};

use path_absolutize::Absolutize;

/// Convert any path to an absolute path, based on the current working
/// directory, without touching the file system.
pub fn normalize_path<P: AsRef<Path>>(path: P) -> PathBuf {
    let path = path.as_ref();
    if let Ok(path) = path.absolutize() {
        return path.to_path_buf();
    }
    path.to_path_buf()
}

/// Canonicalise a path, resolving symlinks and stripping the `\\?\` prefix on
/// Windows. Falls back to [normalize_path] if the path does not exist.
pub fn adjust_canonicalisation<P: AsRef<Path>>(path: P) -> PathBuf {
    let path = path.as_ref();
    dunce::canonicalize(path).unwrap_or_else(|_| normalize_path(path))
}

/// Compute the POSIX form of `path` relative to `root`. Returns `None` if
/// `path` is not inside `root`.
pub fn relative_posix(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    Some(to_posix(relative))
}

/// Render a relative path with `/` separators.
pub fn to_posix(path: &Path) -> String {
    let mut out = String::new();
    for component in path.components() {
        let part = match component {
            Component::Normal(part) => part.to_string_lossy(),
            Component::ParentDir => "..".into(),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => continue,
        };
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(&part);
    }
    out
}

/// Whether any component of a relative POSIX path is `node_modules`.
pub fn is_in_node_modules(posix: &str) -> bool {
    posix.split('/').any(|segment| segment == "node_modules")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_are_posix() {
        let root = Path::new("/repo");
        let path = Path::new("/repo/web/src/app/(dashboard)/page.tsx");
        assert_eq!(relative_posix(root, path).as_deref(), Some("web/src/app/(dashboard)/page.tsx"));
        assert_eq!(relative_posix(root, Path::new("/other/x.ts")), None);
        assert_eq!(relative_posix(root, root).as_deref(), Some(""));
    }

    #[test]
    fn detects_node_modules() {
        assert!(is_in_node_modules("node_modules/react/index.js"));
        assert!(is_in_node_modules("web/node_modules/x/y.js"));
        assert!(!is_in_node_modules("web/src/node_modules_like/x.ts"));
    }
}
