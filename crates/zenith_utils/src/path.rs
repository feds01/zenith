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

/// Whether `extension` is a file extension with its leading `.`, such as `.ts`
/// or `.d.ts`: no path separators, and no empty parts between the `.`s.
pub fn is_extension(extension: &str) -> bool {
    extension.strip_prefix('.').is_some_and(|name| {
        !name.contains(['/', '\\']) && name.split('.').all(|part| !part.is_empty())
    })
}

/// The directory that contains the relative POSIX `path`, which is `""` (the
/// root) for a path with one segment.
pub fn posix_parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

/// The POSIX `path` without the longest of `extensions` (each without its
/// leading `.`) that it ends with, e.g. `src/types` for `src/types.d.ts` given
/// `ts` and `d.ts`. `None` if it ends with none of them, or if its file name
/// is only an extension, e.g. `src/.ts`.
pub fn strip_extension<'p>(path: &'p str, extensions: &[&str]) -> Option<&'p str> {
    // The longest extension leaves the shortest stem.
    let stem = extensions
        .iter()
        .filter_map(|extension| path.strip_suffix(extension)?.strip_suffix('.'))
        .min_by_key(|stem| stem.len())?;
    let name = stem.rsplit('/').next().unwrap_or(stem);
    (!name.is_empty()).then_some(stem)
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
    fn extensions_start_with_a_dot() {
        for extension in [".ts", ".d.ts", ".json", ".module.css"] {
            assert!(is_extension(extension), "{extension:?}");
        }
        for extension in ["", ".", "..", "ts", "d.ts", ".d..ts", ".ts.", "./ts", ".a/b", ".a\\b"] {
            assert!(!is_extension(extension), "{extension:?}");
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

    #[test]
    fn strips_the_longest_extension_given() {
        let extensions = ["ts", "d.ts", "tsx", "cjs"];
        for (path, stem) in [
            ("a/types.ts", Some("a/types")),
            ("a/types.d.ts", Some("a/types")),
            ("a/Button.tsx", Some("a/Button")),
            ("a/.eslintrc.cjs", Some("a/.eslintrc")),
            ("types.ts", Some("types")),
            // Only the extensions given, and only whole ones.
            ("a/styles.css", None),
            ("a/cts", None),
            ("a/Makefile", None),
            // A file name must be more than its extension.
            ("a/.ts", None),
            (".d.ts", None),
        ] {
            assert_eq!(strip_extension(path, &extensions), stem, "{path}");
        }
        assert_eq!(strip_extension("a/types.ts", &[]), None);
    }
}
