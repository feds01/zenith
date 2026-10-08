//! A small resolver for `package.json` `exports` maps, used for workspace
//! packages when `node_modules` is not installed.
//!
//! It handles the shapes used in practice: a string or array shorthand, a
//! conditions object, subpath keys (`"./button"`) and `*` subpath patterns
//! (`"./icons/*": "./src/icons/*.tsx"`). Conditions are matched in the order
//! the package lists them, as Node does.

use serde_json::Value;

/// Resolve `subpath` (`"."` or `"./sub/path"`) through `exports`, returning
/// the target path relative to the package (e.g. `"./src/button.tsx"`).
pub(crate) fn resolve(exports: &Value, subpath: &str, conditions: &[String]) -> Option<String> {
    let is_subpath_map =
        matches!(exports, Value::Object(map) if map.keys().any(|key| key.starts_with('.')));
    if !is_subpath_map {
        return (subpath == ".").then(|| resolve_target(exports, conditions, None)).flatten();
    }
    let Value::Object(map) = exports else { return None };

    if let Some(target) = map.get(subpath) {
        return resolve_target(target, conditions, None);
    }

    // The longest matching pattern wins.
    let mut best: Option<(&str, &str, &Value)> = None;
    for (key, target) in map {
        let Some((prefix, suffix)) = key.split_once('*') else { continue };
        if subpath.len() >= prefix.len() + suffix.len()
            && subpath.starts_with(prefix)
            && subpath.ends_with(suffix)
        {
            let captured = &subpath[prefix.len()..subpath.len() - suffix.len()];
            if best.is_none_or(|(best_key, _, _)| key.len() > best_key.len()) {
                best = Some((key, captured, target));
            }
        }
    }
    best.and_then(|(_, captured, target)| resolve_target(target, conditions, Some(captured)))
}

fn resolve_target(target: &Value, conditions: &[String], captured: Option<&str>) -> Option<String> {
    match target {
        Value::String(path) => Some(match captured {
            Some(captured) => path.replace('*', captured),
            None => path.clone(),
        }),
        Value::Array(items) => {
            items.iter().find_map(|item| resolve_target(item, conditions, captured))
        }
        Value::Object(map) => map
            .iter()
            .filter(|(condition, _)| {
                *condition == "default" || conditions.iter().any(|c| c == *condition)
            })
            .find_map(|(_, value)| resolve_target(value, conditions, captured)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn conditions() -> Vec<String> {
        vec!["types".into(), "import".into()]
    }

    #[test]
    fn resolves_exports_shapes() {
        let c = conditions();
        assert_eq!(resolve(&json!("./index.js"), ".", &c).as_deref(), Some("./index.js"));
        assert_eq!(resolve(&json!("./index.js"), "./x", &c), None);
        assert_eq!(
            resolve(&json!({ "import": "./esm.js", "require": "./cjs.js" }), ".", &c).as_deref(),
            Some("./esm.js")
        );
        let map = json!({
            ".": { "types": "./src/index.ts", "default": "./dist/index.js" },
            "./button": "./src/button.tsx",
            "./icons/*": "./src/icons/*.tsx",
            "./icons/special/*": "./src/special/*.tsx",
            "./internal/*": null
        });
        assert_eq!(resolve(&map, ".", &c).as_deref(), Some("./src/index.ts"));
        assert_eq!(resolve(&map, "./button", &c).as_deref(), Some("./src/button.tsx"));
        assert_eq!(resolve(&map, "./icons/star", &c).as_deref(), Some("./src/icons/star.tsx"));
        assert_eq!(resolve(&map, "./icons/special/x", &c).as_deref(), Some("./src/special/x.tsx"));
        assert_eq!(resolve(&map, "./internal/x", &c), None);
        assert_eq!(resolve(&map, "./missing", &c), None);
    }
}
