//! Path manipulation and canonicalisation utilities.
//!
//! Zenith identifies every internal node by a POSIX path relative to the
//! project root, on every platform.

use std::{
    cmp::Ordering,
    iter,
    path::{Component, MAIN_SEPARATOR, MAIN_SEPARATOR_STR, Path, PathBuf, is_separator},
};

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
    // Zenith names every file it finds and every import it resolves, so take
    // the usual case, a normalised path inside the root, as text.
    relative_text(root, path).or_else(|| relative_by_components(root, path))
}

/// [`relative_posix`] for a UTF-8 `path` that starts with the text of `root`
/// and a separator, and has no empty, `.` or `..` segments after it. `None`
/// for any other path, which [`relative_by_components`] decides on.
fn relative_text(root: &Path, path: &Path) -> Option<String> {
    let (root, path) = (root.to_str()?, path.to_str()?);
    let rest = path.strip_prefix(root)?;
    let rest = if root.ends_with(is_separator) { rest } else { rest.strip_prefix(is_separator)? };
    if rest.split(is_separator).any(|segment| matches!(segment, "" | "." | "..")) {
        return None;
    }
    Some(match MAIN_SEPARATOR {
        '/' => rest.to_owned(),
        _ => rest.replace(is_separator, "/"),
    })
}

/// [`relative_posix`] for any path, comparing it with `root` component by
/// component.
fn relative_by_components(root: &Path, path: &Path) -> Option<String> {
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

/// Order two normalised paths as [`Path`]'s own order does, component by
/// component, but without splitting them into components: it compares their
/// bytes, with separators below every other byte, so a component that ends
/// first sorts first.
///
/// Paths with empty or `.` components, or a trailing separator, which
/// [`Path`]'s order ignores, may be ordered differently.
pub fn compare_paths(a: &Path, b: &Path) -> Ordering {
    let key = |byte: u8| if is_separator(char::from(byte)) { 0 } else { byte };
    let (a, b) = (a.as_os_str().as_encoded_bytes(), b.as_os_str().as_encoded_bytes());
    // Paths of one project share a long prefix, so skip what they share eight
    // bytes at a time, then compare the rest byte by byte.
    let shared = a.len().min(b.len());
    let mut start = 0;
    while start + 8 <= shared && a[start..start + 8] == b[start..start + 8] {
        start += 8;
    }
    let (a, b) = (&a[start..], &b[start..]);
    a.iter().map(|byte| key(*byte)).cmp(b.iter().map(|byte| key(*byte)))
}

/// The absolute path of `name`, a POSIX path relative to `root` such as
/// [`relative_posix`] makes, with the platform's separators.
pub fn absolute_path(root: &Path, name: &str) -> PathBuf {
    // A name is already a path where `/` is the separator, and joining it whole is
    // much cheaper than pushing each of its segments.
    match MAIN_SEPARATOR {
        '/' => root.join(name),
        _ => root.join(name.replace('/', MAIN_SEPARATOR_STR)),
    }
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

/// The relative POSIX directory `dir` and each of its ancestors, nearest
/// first, ending with the root, `""`.
pub fn posix_ancestors(dir: &str) -> impl Iterator<Item = &str> {
    iter::successors(Some(dir), |dir| (!dir.is_empty()).then(|| posix_parent(dir)))
}

/// Join `path`, a relative path as a user wrote it with either separator,
/// onto the relative POSIX directory `base`, resolving its `.` and `..`
/// segments.
///
/// `None` if the result is not inside the root: `path` is absolute, starts
/// with a Windows drive, or climbs out past the root with `..`.
pub fn join_posix(base: &str, path: &str) -> Option<String> {
    if is_absolute_path(path) {
        return None;
    }
    let mut segments: Vec<&str> = base.split('/').filter(|segment| !segment.is_empty()).collect();
    for segment in path.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            segment => segments.push(segment),
        }
    }
    Some(segments.join("/"))
}

/// Whether `path`, as a user wrote it with either separator, is absolute: it
/// starts with a separator or a Windows drive, as in `/srv`, `\\\\srv` or
/// `C:\\work`.
pub fn is_absolute_path(path: &str) -> bool {
    path.starts_with(['/', '\\']) || has_windows_drive(path)
}

/// Whether `path`, as a user wrote it with either separator, is written as a
/// path on the file system rather than a name: it is `.` or `..`, starts with
/// `./` or `../`, or is absolute. Go's `replace` targets and Terraform's module
/// sources tell directories from module names so.
pub fn is_local_path(path: &str) -> bool {
    matches!(path, "." | "..")
        || ["./", "../", ".\\", "..\\"].iter().any(|prefix| path.starts_with(prefix))
        || is_absolute_path(path)
}

/// Whether `path` starts with a Windows drive, as in `C:\\work`.
pub fn has_windows_drive(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
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
    fn paths_are_made_relative_as_text_or_by_components_alike() {
        let roots = ["/repo", "/repo/", "/"];
        let paths = [
            "/repo/web/a.ts",
            "/repo",
            "/repo/",
            "/repository/a.ts",
            "/repo//a.ts",
            "/repo/./a.ts",
            "/repo/a/./b.ts",
            "/repo/a/../b.ts",
            "/repo/a/",
            "/other/a.ts",
            "repo/a.ts",
        ];
        for root in roots {
            for path in paths {
                let (root, path) = (Path::new(root), Path::new(path));
                let expected = relative_by_components(root, path);
                assert_eq!(relative_posix(root, path), expected, "{root:?} {path:?}");
                // The text form only answers when it agrees.
                if let Some(text) = relative_text(root, path) {
                    assert_eq!(Some(text), expected, "{root:?} {path:?}");
                }
            }
        }
        assert_eq!(
            relative_text(Path::new("/repo"), Path::new("/repo/a/b.ts")).as_deref(),
            Some("a/b.ts")
        );
    }

    #[test]
    fn paths_are_compared_in_the_order_of_their_components() {
        let paths = [
            "/repo",
            "/repo/a",
            "/repo/a/b.ts",
            "/repo/a-b.ts",
            "/repo/a.ts",
            "/repo/a0/b.ts",
            "/repo/ab/c.ts",
            "/repo/B/c.ts",
            "/repo/a/b/c.ts",
            "/repo/a/b-c.ts",
            "/repo/(group)/[id].tsx",
            "/repo/é/a.ts",
            "/other",
            "/",
        ];
        for a in paths {
            for b in paths {
                let (a, b) = (Path::new(a), Path::new(b));
                assert_eq!(compare_paths(a, b), a.cmp(b), "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn names_relative_to_a_root_are_made_absolute_again() {
        let root = Path::new("/repo");
        for name in ["a.ts", "web/src/(group)/[id]/page.tsx"] {
            let path = absolute_path(root, name);
            assert_eq!(relative_posix(root, &path).as_deref(), Some(name));
        }
        assert_eq!(absolute_path(root, "web/a.ts"), root.join("web").join("a.ts"));
        assert_eq!(absolute_path(root, ""), root);
        assert_eq!(absolute_path(Path::new("/"), "a/b.ts"), Path::new("/").join("a").join("b.ts"));
        assert_eq!(absolute_path(Path::new("/repo/"), "a.ts"), Path::new("/repo/a.ts"));
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

    #[test]
    fn posix_directories_list_their_ancestors() {
        assert_eq!(posix_ancestors("a/b/c").collect::<Vec<_>>(), ["a/b/c", "a/b", "a", ""]);
        assert_eq!(posix_ancestors("").collect::<Vec<_>>(), [""]);
    }

    #[test]
    fn relative_paths_join_onto_a_posix_directory() {
        assert_eq!(join_posix("", "./a").as_deref(), Some("a"));
        assert_eq!(join_posix("svc/api", "../shared").as_deref(), Some("svc/shared"));
        assert_eq!(join_posix("svc/api", "..\\..\\lib/x").as_deref(), Some("lib/x"));
        assert_eq!(join_posix("svc", "..").as_deref(), Some(""));
        assert_eq!(join_posix("svc", ".").as_deref(), Some("svc"));
        assert_eq!(join_posix("a", "b//c/./d/../e").as_deref(), Some("a/b/c/e"));
    }

    #[test]
    fn paths_outside_the_root_do_not_join() {
        assert_eq!(join_posix("", ".."), None);
        assert_eq!(join_posix("svc", "../.."), None);
        assert_eq!(join_posix("svc", "/etc"), None);
        assert_eq!(join_posix("svc", "C:\\x"), None);
        assert_eq!(join_posix("svc", "\\\\srv\\x"), None);
    }

    #[test]
    fn local_paths_are_told_from_names() {
        for local in
            [".", "..", "./a", "../a", ".\\a", "..\\a", "/srv/a", "\\\\srv\\a", "C:/a", "c:\\a"]
        {
            assert!(is_local_path(local), "{local}");
        }
        for name in ["a", "a/b", ".a", "...", "example.com/a", ""] {
            assert!(!is_local_path(name), "{name}");
        }
        assert!(is_absolute_path("/a") && is_absolute_path("C:/a") && !is_absolute_path("./a"));
    }

    #[test]
    fn windows_drives_are_recognised() {
        assert!(has_windows_drive("C:\\work") && has_windows_drive("c:/work"));
        assert!(!has_windows_drive("c") && !has_windows_drive("./c:") && !has_windows_drive(""));
    }
}
