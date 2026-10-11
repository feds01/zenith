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
    match path.absolutize() {
        Ok(path) => path.into_owned(),
        Err(_) => path.to_path_buf(),
    }
}

/// Canonicalise a path: resolve symlinks and, on Windows, strip the `\\?\`
/// prefix where that is safe. A path that does not exist is normalised
/// lexically with [`normalize_path`] instead, so symlinks in its existing
/// ancestors stay unresolved.
pub fn adjust_canonicalisation<P: AsRef<Path>>(path: P) -> PathBuf {
    let path = path.as_ref();
    dunce::canonicalize(path).unwrap_or_else(|_| normalize_path(path))
}

/// The POSIX form of `path` relative to `root`, or `None` if `path` is not
/// inside `root`. A `..` component counts as outside, since it can lead out of
/// `root`.
pub fn relative_posix(root: &Path, path: &Path) -> Option<String> {
    let mut out = String::new();
    for component in path.strip_prefix(root).ok()?.components() {
        let part = match component {
            Component::Normal(part) => part.to_string_lossy(),
            Component::CurDir => continue,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        };
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(&part);
    }
    Some(out)
}

/// Whether `path` is a normalised POSIX path relative to a root, such as
/// [`relative_posix`] makes: one or more segments separated by single `/`s,
/// none of them `.` or `..`. The root itself, `""`, is not.
pub fn is_relative_posix(path: &str) -> bool {
    !path.is_empty() && path.split('/').all(|segment| !matches!(segment, "" | "." | ".."))
}

/// The directory that contains the relative POSIX `path`, which is `""` (the
/// root) for a path with one segment.
pub fn posix_parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    #[test]
    fn relative_paths_are_posix() {
        let root = Path::new("/repo");
        let path = Path::new("/repo/web/src/app/(dashboard)/page.tsx");
        assert_eq!(relative_posix(root, path).as_deref(), Some("web/src/app/(dashboard)/page.tsx"));
        assert_eq!(relative_posix(root, Path::new("/other/x.ts")), None);
        assert_eq!(relative_posix(root, Path::new("/repo/../etc/passwd")), None);
        assert_eq!(relative_posix(root, root).as_deref(), Some(""));
    }

    #[test]
    fn relative_posix_paths_are_normalised() {
        for path in ["a.ts", "web/src/app.ts", "web/(group)/.hidden/page.tsx"] {
            assert!(is_relative_posix(path), "{path:?}");
        }
        for path in ["", "/a.ts", "a/", "a//b.ts", "./a.ts", "a/./b.ts", "a/../b.ts", ".", ".."] {
            assert!(!is_relative_posix(path), "{path:?}");
        }
    }

    #[test]
    fn the_parent_of_a_top_level_path_is_the_root() {
        assert_eq!(posix_parent("web/src/app.ts"), "web/src");
        assert_eq!(posix_parent("web"), "");
        assert_eq!(posix_parent(""), "");
    }

    #[cfg(windows)]
    #[test]
    fn windows_separators_become_slashes() {
        let root = Path::new(r"C:\repo");
        assert_eq!(
            relative_posix(root, Path::new(r"C:\repo\web\a.ts")).as_deref(),
            Some("web/a.ts")
        );
    }

    #[test]
    fn existing_paths_are_canonicalised() {
        assert_eq!(adjust_canonicalisation("."), env::current_dir().unwrap());
    }

    #[test]
    fn missing_paths_are_normalised() {
        let cwd = env::current_dir().unwrap();
        assert_eq!(adjust_canonicalisation("no-such-dir/x/../y.ts"), cwd.join("no-such-dir/y.ts"));
    }
}
