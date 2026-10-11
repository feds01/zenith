//! What can be told about a specifier, or a resolved path, from its text
//! alone.

use std::path::Path;

/// A builtin without its subpath, so `node:fs/promises` is `node:fs`.
pub(crate) fn squash_builtin(builtin: &str) -> &str {
    builtin.split_once('/').map_or(builtin, |(name, _)| name)
}

/// The package that a bare specifier imports from: `@scope/name/sub` is
/// `@scope/name` and `name/sub` is `name`.
///
/// `None` for anything else: relative and absolute specifiers, `#` subpath
/// imports, URLs, and path aliases that can't be package names, such as
/// `@/lib` or `~/lib`. A package name and its scope start with an ASCII letter
/// or digit, followed by letters, digits, `-`, `.`, `_` or `~`.
pub(crate) fn package_name(specifier: &str) -> Option<&str> {
    let end = match specifier.strip_prefix('@') {
        Some(scoped) => {
            let (scope, rest) = scoped.split_once('/')?;
            let name = rest.split_once('/').map_or(rest, |(name, _)| name);
            if !is_name_part(scope) || !is_name_part(name) {
                return None;
            }
            1 + scope.len() + 1 + name.len()
        }
        None => {
            let name = specifier.split_once('/').map_or(specifier, |(name, _)| name);
            if !is_name_part(name) {
                return None;
            }
            name.len()
        }
    };
    Some(&specifier[..end])
}

/// Whether `part` can be a package name, or the scope of one, without `@`.
fn is_name_part(part: &str) -> bool {
    part.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
}

/// The package that a resolved path belongs to, named by the segments after
/// its last `node_modules` segment.
pub(crate) fn package_from_path(path: &Path) -> Option<String> {
    // Walk backwards, remembering the two segments after the current one.
    let (mut first, mut second): (Option<&str>, Option<&str>) = (None, None);
    for component in path.components().rev() {
        let segment = component.as_os_str();
        if segment == "node_modules" {
            let first = first?;
            return match first {
                scope if scope.starts_with('@') => Some(format!("{scope}/{}", second?)),
                name => Some(name.to_string()),
            };
        }
        second = first;
        first = segment.to_str();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squashes_builtin_subpaths() {
        assert_eq!(squash_builtin("node:fs/promises"), "node:fs");
        assert_eq!(squash_builtin("node:fs"), "node:fs");
        assert_eq!(squash_builtin("bun:sqlite"), "bun:sqlite");
    }

    #[test]
    fn extracts_package_names() {
        assert_eq!(package_name("react"), Some("react"));
        assert_eq!(package_name("lodash/get"), Some("lodash"));
        assert_eq!(package_name("lodash.get"), Some("lodash.get"));
        assert_eq!(package_name("@tanstack/react-query"), Some("@tanstack/react-query"));
        assert_eq!(package_name("@acme/ui/src/button"), Some("@acme/ui"));
        assert_eq!(package_name("JSONStream"), Some("JSONStream"));
    }

    #[test]
    fn rejects_specifiers_that_are_not_packages() {
        for specifier in [
            "",
            "./a",
            "../a",
            "/a",
            "#internal",
            "node:fs",
            "virtual:pwa",
            "@scope",
            "@scope/",
            "@/lib/format",
            "~/lib/format",
            "~icons/mdi/home",
            "$lib/format",
            ".hidden",
            "_private",
        ] {
            assert_eq!(package_name(specifier), None, "{specifier}");
        }
    }

    #[test]
    fn extracts_packages_from_paths() {
        let package = |path: &str| package_from_path(Path::new(path));
        assert_eq!(
            package("/r/node_modules/.bun/x@1/node_modules/@scope/pkg/dist/index.js").as_deref(),
            Some("@scope/pkg")
        );
        assert_eq!(package("/r/node_modules/react/index.js").as_deref(), Some("react"));
        assert_eq!(package("/r/node_modules/@scope"), None);
        assert_eq!(package("/r/node_modules"), None);
        assert_eq!(package("/r/src/a.ts"), None);
    }
}
