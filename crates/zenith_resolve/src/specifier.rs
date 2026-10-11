//! What can be told about a specifier, or a resolved path, from its text
//! alone.

use std::path::Path;

/// Split a URL scheme off a specifier, so `virtual:pwa-register` is
/// `("virtual", "pwa-register")`. A scheme starts with a lowercase ASCII
/// letter, followed by lowercase letters, digits, `+`, `-` or `.`. It is at
/// least two characters long, so a Windows drive letter is not a scheme.
pub(crate) fn split_scheme(specifier: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = specifier.split_once(':')?;
    let mut bytes = scheme.bytes();
    let valid = scheme.len() >= 2
        && bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.')
        });
    valid.then_some((scheme, rest))
}

/// Split a specifier into its path and its `?query` or `#fragment`, which
/// bundlers use for variants of a file, as in `./icon.svg?react`. A leading `#`
/// starts a subpath import, not a fragment.
pub(crate) fn split_query(specifier: &str) -> (&str, &str) {
    match specifier.get(1..).and_then(|rest| rest.find(['?', '#'])) {
        Some(index) => specifier.split_at(index + 1),
        None => (specifier, ""),
    }
}

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
    fn splits_schemes() {
        assert_eq!(split_scheme("node:fs"), Some(("node", "fs")));
        assert_eq!(split_scheme("bun:sqlite"), Some(("bun", "sqlite")));
        assert_eq!(split_scheme("virtual:pwa-register"), Some(("virtual", "pwa-register")));
        assert_eq!(split_scheme("astro:content"), Some(("astro", "content")));
        assert_eq!(split_scheme("c:/repo/a.ts"), None);
        assert_eq!(split_scheme(r"C:\repo\a.ts"), None);
        assert_eq!(split_scheme("@scope/pkg:sub"), None);
        assert_eq!(split_scheme("./a:b"), None);
        assert_eq!(split_scheme("react"), None);
    }

    #[test]
    fn splits_queries_and_fragments() {
        assert_eq!(split_query("./icon.svg?react"), ("./icon.svg", "?react"));
        assert_eq!(split_query("@acme/ui/styles.css?inline"), ("@acme/ui/styles.css", "?inline"));
        assert_eq!(split_query("@acme/ui#main"), ("@acme/ui", "#main"));
        assert_eq!(split_query("#internal?raw"), ("#internal", "?raw"));
        assert_eq!(split_query("#internal"), ("#internal", ""));
        assert_eq!(split_query("react"), ("react", ""));
        assert_eq!(split_query(""), ("", ""));
    }

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
