//! `package.json` `exports` maps, for resolving workspace packages without
//! `node_modules`.
//!
//! This follows Node's algorithm for the shapes used in practice:
//!
//! - A string, an array or an object of conditions exports the package's main
//!   entry (`.`) only.
//! - Otherwise the keys are subpaths (`"./button"`), subpath patterns with one
//!   `*` (`"./icons/*": "./src/icons/*.tsx"`), or folders ending in `/`
//!   (`"./feat/": "./src/feat/"`), which Node no longer supports but TypeScript
//!   and bundlers still do. `*` matches a non-empty string, `/` included.
//! - An exact key wins. Otherwise the most specific matching key wins: the one
//!   with the longest prefix before its `*` (or the longest folder), then a
//!   pattern over a folder, then the longest key.
//! - Conditions are matched in the order the package lists them, not the order
//!   they are configured in, and `default` always matches.
//! - `null` excludes a subpath. So does a target that doesn't stay inside the
//!   package: it must start with `./`, and have no empty, `.`, `..` or
//!   `node_modules` segments after that.

use std::borrow::Cow;

use serde_json::{Map, Value};

/// Resolve `subpath` (`"."` or `"./sub/path"`) through `exports`, to a target
/// relative to the package, e.g. `"./src/button.tsx"`. `None` means that the
/// subpath is not exported.
pub(crate) fn resolve<'a>(
    exports: &'a Value,
    subpath: &str,
    conditions: &[String],
) -> Option<Cow<'a, str>> {
    match exports {
        Value::Object(map) if map.keys().any(|key| key.starts_with('.')) => {
            resolve_subpath(map, subpath, conditions)
        }
        _ if subpath == "." => resolve_target(exports, Substitution::None, conditions).into_path(),
        _ => None,
    }
}

/// Resolve `subpath` through a map of subpaths, patterns and folders.
fn resolve_subpath<'a>(
    map: &'a Map<String, Value>,
    subpath: &str,
    conditions: &[String],
) -> Option<Cow<'a, str>> {
    if !subpath.contains('*')
        && let Some(target) = map.get(subpath)
    {
        return resolve_target(target, Substitution::None, conditions).into_path();
    }

    // The most specific match, with its rank: the length of the prefix up to
    // and including any `*`, whether it is a pattern, and the key's length.
    let mut best: Option<((usize, bool, usize), Substitution, &Value)> = None;
    for (key, target) in map {
        let (rank, substitution) = match key.split_once('*') {
            Some((_, suffix)) if suffix.contains('*') => continue,
            Some((prefix, suffix)) => {
                match subpath.strip_prefix(prefix).and_then(|rest| rest.strip_suffix(suffix)) {
                    Some(captured) => ((prefix.len() + 1, true), Substitution::Pattern(captured)),
                    None => continue,
                }
            }
            None if key.ends_with('/') => match subpath.strip_prefix(key.as_str()) {
                Some(rest) => ((key.len(), false), Substitution::Folder(rest)),
                None => continue,
            },
            None => continue,
        };
        let rank = (rank.0, rank.1, key.len());
        if !substitution.is_empty() && best.as_ref().is_none_or(|(best, ..)| rank > *best) {
            best = Some((rank, substitution, target));
        }
    }
    let (_, substitution, target) = best?;
    resolve_target(target, substitution, conditions).into_path()
}

/// How the target of a matched key becomes a path.
#[derive(Debug, Clone, Copy)]
enum Substitution<'s> {
    /// The key matched exactly, so the target is the path.
    None,

    /// A `*` pattern matched this text, which replaces every `*` in the
    /// target.
    Pattern(&'s str),

    /// A folder matched, and this is the rest of the subpath, which is
    /// appended to the target. The target must be a folder too.
    Folder(&'s str),
}

impl Substitution<'_> {
    /// Whether the key matched nothing beyond its literal text.
    fn is_empty(self) -> bool {
        match self {
            Self::None => true,
            Self::Pattern(text) | Self::Folder(text) => text.is_empty(),
        }
    }
}

/// The outcome of resolving a target.
enum Target<'a> {
    /// A path inside the package.
    Path(Cow<'a, str>),

    /// No condition matched, so the next condition is tried.
    NoMatch,

    /// `null`, or an invalid target: the subpath is not exported.
    Excluded,
}

impl<'a> Target<'a> {
    /// The path, if there is one.
    fn into_path(self) -> Option<Cow<'a, str>> {
        match self {
            Self::Path(path) => Some(path),
            Self::NoMatch | Self::Excluded => None,
        }
    }
}

/// Resolve a target, completing it with what the key matched.
fn resolve_target<'a>(
    target: &'a Value,
    substitution: Substitution,
    conditions: &[String],
) -> Target<'a> {
    match target {
        Value::String(template) => {
            let path = match substitution {
                Substitution::None => Cow::Borrowed(template.as_str()),
                Substitution::Pattern(captured) => Cow::Owned(template.replace('*', captured)),
                Substitution::Folder(rest) if template.ends_with('/') => {
                    Cow::Owned(format!("{template}{rest}"))
                }
                Substitution::Folder(_) => return Target::Excluded,
            };
            if stays_inside_package(&path) { Target::Path(path) } else { Target::Excluded }
        }
        // The first valid target wins. Items that match no condition are
        // skipped, and if every item is, so is the array.
        Value::Array(items) => {
            let mut excluded = items.is_empty();
            for item in items {
                match resolve_target(item, substitution, conditions) {
                    path @ Target::Path(_) => return path,
                    Target::Excluded => excluded = true,
                    Target::NoMatch => {}
                }
            }
            if excluded { Target::Excluded } else { Target::NoMatch }
        }
        Value::Object(map) => {
            for (condition, value) in map {
                if condition != "default" && !conditions.iter().any(|c| c == condition) {
                    continue;
                }
                match resolve_target(value, substitution, conditions) {
                    Target::NoMatch => {}
                    outcome => return outcome,
                }
            }
            Target::NoMatch
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => Target::Excluded,
    }
}

/// Whether a target stays inside its package.
fn stays_inside_package(target: &str) -> bool {
    target.strip_prefix("./").is_some_and(|rest| {
        rest.split(['/', '\\']).all(|segment| {
            !matches!(segment, "" | "." | "..") && !segment.eq_ignore_ascii_case("node_modules")
        })
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn conditions(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    fn resolve_with(exports: &Value, subpath: &str, names: &[&str]) -> Option<String> {
        resolve(exports, subpath, &conditions(names)).map(Cow::into_owned)
    }

    #[test]
    fn shorthands_export_the_main_entry_only() {
        let names = ["types", "import"];
        assert_eq!(resolve_with(&json!("./index.js"), ".", &names).as_deref(), Some("./index.js"));
        assert_eq!(resolve_with(&json!("./index.js"), "./x", &names), None);
        assert_eq!(
            resolve_with(&json!(["./index.js", "./fallback.js"]), ".", &names).as_deref(),
            Some("./index.js")
        );
        assert_eq!(
            resolve_with(&json!({ "import": "./esm.js", "require": "./cjs.js" }), ".", &names)
                .as_deref(),
            Some("./esm.js")
        );
    }

    #[test]
    fn conditions_match_in_package_order() {
        let exports = json!({ "require": "./cjs.js", "import": "./esm.js" });
        assert_eq!(
            resolve_with(&exports, ".", &["import", "require"]).as_deref(),
            Some("./cjs.js")
        );
        assert_eq!(resolve_with(&exports, ".", &["import"]).as_deref(), Some("./esm.js"));
        assert_eq!(resolve_with(&exports, ".", &["types"]), None);
    }

    #[test]
    fn default_always_matches_and_nested_misses_fall_through() {
        let exports = json!({
            "node": { "worker": "./worker.js" },
            "browser": "./browser.js",
            "default": "./index.js"
        });
        assert_eq!(resolve_with(&exports, ".", &["node"]).as_deref(), Some("./index.js"));
        assert_eq!(
            resolve_with(&exports, ".", &["node", "worker"]).as_deref(),
            Some("./worker.js")
        );
        assert_eq!(resolve_with(&exports, ".", &["browser"]).as_deref(), Some("./browser.js"));
        assert_eq!(resolve_with(&exports, ".", &[]).as_deref(), Some("./index.js"));
    }

    #[test]
    fn subpaths_and_patterns() {
        let names = ["types", "import"];
        let exports = json!({
            ".": { "types": "./src/index.ts", "default": "./dist/index.js" },
            "./button": "./src/button.tsx",
            "./icons/*": "./src/icons/*.tsx",
            "./icons/special/*": "./src/special/*.tsx",
            "./internal/*": null,
            "./*": "./src/*.ts"
        });
        let resolved = |subpath: &str| resolve_with(&exports, subpath, &names);
        assert_eq!(resolved(".").as_deref(), Some("./src/index.ts"));
        assert_eq!(resolved("./button").as_deref(), Some("./src/button.tsx"));
        assert_eq!(resolved("./icons/star").as_deref(), Some("./src/icons/star.tsx"));
        assert_eq!(
            resolved("./icons/outline/star").as_deref(),
            Some("./src/icons/outline/star.tsx")
        );
        assert_eq!(resolved("./icons/special/x").as_deref(), Some("./src/special/x.tsx"));
        assert_eq!(resolved("./utils").as_deref(), Some("./src/utils.ts"));
        // `null` excludes the subpath, even though a less specific pattern
        // matches it.
        assert_eq!(resolved("./internal/x"), None);
    }

    #[test]
    fn stars_match_non_empty_strings() {
        let exports = json!({ "./icons/*": "./src/icons/*.tsx" });
        assert_eq!(resolve_with(&exports, "./icons/", &[]), None);
        assert_eq!(resolve_with(&exports, "./icons/a", &[]).as_deref(), Some("./src/icons/a.tsx"));
    }

    #[test]
    fn the_longest_prefix_wins_over_the_longest_key() {
        let exports = json!({ "./*.js": "./by-suffix/*.js", "./a/*": "./by-prefix/*" });
        assert_eq!(resolve_with(&exports, "./a/x.js", &[]).as_deref(), Some("./by-prefix/x.js"));
        assert_eq!(resolve_with(&exports, "./b/x.js", &[]).as_deref(), Some("./by-suffix/b/x.js"));
    }

    #[test]
    fn every_star_in_a_target_is_replaced() {
        let exports = json!({ "./locales/*": "./src/*/*.json" });
        assert_eq!(
            resolve_with(&exports, "./locales/en", &[]).as_deref(),
            Some("./src/en/en.json")
        );
    }

    #[test]
    fn patterns_with_two_stars_are_ignored() {
        let exports = json!({ "./*/x/*": "./src/*.ts" });
        assert_eq!(resolve_with(&exports, "./a/x/b", &[]), None);
    }

    #[test]
    fn targets_must_stay_inside_the_package() {
        for target in [
            "../other/index.js",
            "dist/index.js",
            "./a/../../x.js",
            "./node_modules/x.js",
            "/abs.js",
            "./",
        ] {
            assert_eq!(resolve_with(&json!(target), ".", &[]), None, "{target}");
        }
        let exports = json!({ "./*": "./src/*" });
        assert_eq!(resolve_with(&exports, "./../secret", &[]), None);
        assert_eq!(resolve_with(&exports, "./a//b", &[]), None);
        // An invalid target in an array falls back to the next one.
        assert_eq!(
            resolve_with(&json!(["dist/index.js", "./index.js"]), ".", &[]).as_deref(),
            Some("./index.js")
        );
    }

    #[test]
    fn folders_map_the_rest_of_the_subpath() {
        let exports = json!({
            "./feat/": "./src/feat/",
            "./feat/*.css": "./styles/*.css",
            "./bad/": "./src/bad"
        });
        assert_eq!(
            resolve_with(&exports, "./feat/a/b.ts", &[]).as_deref(),
            Some("./src/feat/a/b.ts")
        );
        // A pattern with the same prefix is more specific than a folder.
        assert_eq!(resolve_with(&exports, "./feat/a.css", &[]).as_deref(), Some("./styles/a.css"));
        assert_eq!(resolve_with(&exports, "./feat/", &[]), None);
        // A folder's target must be a folder.
        assert_eq!(resolve_with(&exports, "./bad/a.ts", &[]), None);
    }

    #[test]
    fn arrays_that_match_no_condition_are_skipped() {
        let exports =
            json!({ ".": { "import": [{ "worker": "./worker.ts" }], "default": "./index.ts" } });
        assert_eq!(resolve_with(&exports, ".", &["import"]).as_deref(), Some("./index.ts"));
        assert_eq!(
            resolve_with(&exports, ".", &["import", "worker"]).as_deref(),
            Some("./worker.ts")
        );
        let exports = json!({ ".": { "import": [null], "default": "./index.ts" } });
        assert_eq!(resolve_with(&exports, ".", &["import"]), None);
    }

    #[test]
    fn unexported_subpaths() {
        let exports = json!({ ".": "./index.js", "./button": "./button.js" });
        assert_eq!(resolve_with(&exports, "./missing", &[]), None);
        assert_eq!(resolve_with(&json!({ "./x": { "import": "./x.js" } }), "./x", &[]), None);
        assert_eq!(resolve_with(&json!({ ".": null }), ".", &[]), None);
        assert_eq!(resolve_with(&json!([]), ".", &[]), None);
        assert_eq!(resolve_with(&json!(true), ".", &[]), None);
    }
}
