//! Lexical helpers for the root-relative POSIX directories that the Go
//! support works with, and for the local paths that `replace` and `use` name.

use std::iter;

use zenith_utils::path::posix_parent;

/// Whether `path` starts with a Windows drive, as in `C:\work`.
fn has_drive(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Whether a `replace` target is a path on disk rather than a module path: it
/// starts with `./`, `../` or `/`, is `.` or `..`, or is a Windows path, such
/// as `..\shared` or `C:\work\shared`.
pub(crate) fn is_local_path(path: &str) -> bool {
    matches!(path, "." | "..")
        || path.starts_with(['/', '\\'])
        || path.starts_with("./")
        || path.starts_with("../")
        || path.starts_with(".\\")
        || path.starts_with("..\\")
        || has_drive(path)
}

/// Join `rel`, a local path as written in a `go.mod` or `go.work`, onto the
/// root-relative directory `base`, normalising `.` and `..` segments, and
/// either separator.
///
/// `None` if the result is not inside the root: `rel` is absolute, or its `..`
/// segments climb out of `base` past the root.
pub(crate) fn join_clean(base: &str, rel: &str) -> Option<String> {
    if rel.starts_with(['/', '\\']) || has_drive(rel) {
        return None;
    }
    let mut segments: Vec<&str> = base.split('/').filter(|segment| !segment.is_empty()).collect();
    for segment in rel.split(['/', '\\']) {
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

/// `dir` and each of its ancestors, ending with the root, `""`.
pub(crate) fn ancestors(dir: &str) -> impl Iterator<Item = &str> {
    iter::successors(Some(dir), |dir| (!dir.is_empty()).then(|| posix_parent(dir)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_paths_are_told_from_module_paths() {
        for local in [".", "..", "./a", "../a", "/abs", "..\\a", ".\\a", "C:\\a", "c:/a", "\\\\srv"]
        {
            assert!(is_local_path(local), "{local}");
        }
        for module in ["a", "example.com/a", "github.com/x/y", "c", ".hidden", "..a", ""] {
            assert!(!is_local_path(module), "{module}");
        }
    }

    #[test]
    fn local_paths_join_onto_a_directory() {
        assert_eq!(join_clean("", "./a").as_deref(), Some("a"));
        assert_eq!(join_clean("svc/api", "../shared").as_deref(), Some("svc/shared"));
        assert_eq!(join_clean("svc/api", "..\\..\\lib/x").as_deref(), Some("lib/x"));
        assert_eq!(join_clean("svc", "..").as_deref(), Some(""));
        assert_eq!(join_clean("svc", ".").as_deref(), Some("svc"));
        assert_eq!(join_clean("a", "b//c/./d/../e").as_deref(), Some("a/b/c/e"));
    }

    #[test]
    fn local_paths_outside_the_root_do_not_join() {
        assert_eq!(join_clean("", ".."), None);
        assert_eq!(join_clean("svc", "../.."), None);
        assert_eq!(join_clean("svc", "/etc"), None);
        assert_eq!(join_clean("svc", "C:\\x"), None);
        assert_eq!(join_clean("svc", "\\\\srv\\x"), None);
    }

    #[test]
    fn directories_list_their_ancestors() {
        assert_eq!(ancestors("a/b/c").collect::<Vec<_>>(), ["a/b/c", "a/b", "a", ""]);
        assert_eq!(ancestors("").collect::<Vec<_>>(), [""]);
    }
}
