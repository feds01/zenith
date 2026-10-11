//! The JSON Schema of `zenith.toml` describes what parsing accepts: the
//! documented example passes both, and unknown keys and values fail both.

use serde_json::{Value, json};

use super::json_schema;
use crate::{config::Config, contracts::CONTRACT_TYPES};

/// The example configuration of the docs, with every table and contract
/// type.
const EXAMPLE: &str = include_str!("example.toml");

/// The keywords that [`validate`] understands. The schema must not use any
/// other, so that nothing it says is skipped.
const KEYWORDS: [&str; 16] = [
    "$schema",
    "$defs",
    "$ref",
    "title",
    "description",
    "default",
    "format",
    "type",
    "const",
    "minimum",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "oneOf",
    "anyOf",
];

/// Check `value` against `schema`, for the keywords in [`KEYWORDS`], and
/// collect what is wrong as `<JSON pointer>: <reason>`.
fn validate(root: &Value, schema: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
    let Some(schema) = schema.as_object() else {
        if schema == &Value::Bool(false) {
            errors.push(format!("{path}: not allowed"));
        }
        return;
    };
    for keyword in schema.keys() {
        assert!(KEYWORDS.contains(&keyword.as_str()), "unsupported keyword `{keyword}`");
    }

    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let name = reference.strip_prefix("#/$defs/").expect("a local reference");
        validate(root, &root["$defs"][name], value, path, errors);
    }

    if let Some(expected) = schema.get("type") {
        let expected = expected.as_str().expect("a single type");
        let matches = match expected {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
            "integer" => value.is_i64() || value.is_u64(),
            other => panic!("unexpected type `{other}`"),
        };
        if !matches {
            errors.push(format!("{path}: expected {expected}, got {value}"));
            return;
        }
    }
    if let Some(constant) = schema.get("const")
        && value != constant
    {
        errors.push(format!("{path}: expected {constant}, got {value}"));
    }
    if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64)
        && value.as_f64().is_some_and(|number| number < minimum)
    {
        errors.push(format!("{path}: {value} is less than {minimum}"));
    }

    if let Some(object) = value.as_object() {
        for key in schema.get("required").and_then(Value::as_array).into_iter().flatten() {
            let key = key.as_str().expect("a key");
            if !object.contains_key(key) {
                errors.push(format!("{path}: missing `{key}`"));
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (key, item) in object {
            let item_path = format!("{path}/{key}");
            match (
                properties.and_then(|properties| properties.get(key)),
                schema.get("additionalProperties"),
            ) {
                (Some(property), _) => validate(root, property, item, &item_path, errors),
                (None, Some(additional)) => validate(root, additional, item, &item_path, errors),
                (None, None) => {}
            }
        }
    }
    if let (Some(items), Some(item_schema)) = (value.as_array(), schema.get("items")) {
        for (index, item) in items.iter().enumerate() {
            validate(root, item_schema, item, &format!("{path}/{index}"), errors);
        }
    }

    for (keyword, exactly_one) in [("oneOf", true), ("anyOf", false)] {
        let Some(branches) = schema.get(keyword).and_then(Value::as_array) else {
            continue;
        };
        let matching = branches
            .iter()
            .filter(|branch| {
                let mut branch_errors = Vec::new();
                validate(root, branch, value, path, &mut branch_errors);
                branch_errors.is_empty()
            })
            .count();
        if matching == 0 || (exactly_one && matching > 1) {
            errors.push(format!("{path}: matches {matching} of the `{keyword}` schemas"));
        }
    }
}

/// What is wrong with a configuration, given as JSON, according to the
/// schema.
fn schema_errors(config: &Value) -> Vec<String> {
    let schema = json_schema();
    let mut errors = Vec::new();
    validate(&schema, &schema, config, "", &mut errors);
    errors
}

/// The example configuration as JSON.
fn example() -> Value {
    toml::from_str(EXAMPLE).expect("the example is valid TOML")
}

/// Every subschema of a schema, including itself.
fn subschemas(schema: &Value) -> Vec<&Value> {
    let mut found = vec![schema];
    let mut index = 0;
    while let Some(schema) = found.get(index).copied() {
        index += 1;
        let Some(object) = schema.as_object() else { continue };
        for (keyword, value) in object {
            match (keyword.as_str(), value) {
                ("properties" | "$defs", Value::Object(map)) => found.extend(map.values()),
                ("oneOf" | "anyOf", Value::Array(branches)) => found.extend(branches),
                ("items" | "additionalProperties", Value::Object(_)) => found.push(value),
                _ => {}
            }
        }
    }
    found
}

#[test]
fn the_example_is_valid() {
    let config = Config::parse(EXAMPLE).expect("the example parses");
    let types: Vec<_> = config.contracts.iter().map(|contract| contract.kind.type_name()).collect();
    assert_eq!(
        types,
        ["forbidden", "independence", "layers", "layers", "protected", "acyclic", "encapsulation"]
    );
    assert_eq!(schema_errors(&example()), Vec::<String>::new());
}

#[test]
fn the_schema_rejects_what_parsing_rejects() {
    /// A change to the example that makes it invalid.
    type Mutation = fn(&mut Value);

    let mutations: [(&str, Mutation); 9] = [
        ("an unknown table", |config| config["filez"] = json!({})),
        ("an unknown key in a table", |config| config["files"]["exclud"] = json!([])),
        ("an unknown key in a contract", |config| {
            config["contracts"]["client-safety"]["fromm"] = json!([]);
        }),
        ("a key of another contract type", |config| {
            config["contracts"]["client-safety"]["modules"] = json!([]);
        }),
        ("an unknown key in a selector", |config| {
            config["contracts"]["client-safety"]["from"][0]["paths"] = json!("a");
        }),
        ("an unknown key in a layer", |config| {
            config["contracts"]["package-tiers"]["layers"][2]["close"] = json!(true);
        }),
        ("an unknown contract type", |config| {
            config["contracts"]["client-safety"]["type"] = json!("layer");
        }),
        ("an unknown severity", |config| {
            config["contracts"]["ui-public-surface"]["severity"] = json!("fatal");
        }),
        ("a missing required key", |config| {
            if let Some(contract) = config["contracts"]["agents-are-api-only"].as_object_mut() {
                contract.remove("allowed_importers");
            }
        }),
    ];

    for (what, mutate) in mutations {
        let mut config = example();
        mutate(&mut config);
        assert!(!schema_errors(&config).is_empty(), "the schema accepts {what}");
        let toml = toml::to_string(&config).expect("the config serialises");
        assert!(Config::parse(&toml).is_err(), "parsing accepts {what}");
    }
}

#[test]
fn the_schema_lists_every_contract_type() {
    let schema = json_schema();
    let types: Vec<_> = schema["$defs"]["ContractTable"]["oneOf"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|branch| branch["properties"]["type"]["const"].as_str())
        .collect();
    assert_eq!(types, CONTRACT_TYPES.map(Some));
}

#[test]
fn every_table_rejects_unknown_keys() {
    let schema = json_schema();
    for subschema in subschemas(&schema) {
        if subschema.get("properties").is_some() {
            assert_eq!(subschema.get("additionalProperties"), Some(&json!(false)), "{subschema:#}");
        }
    }
}

#[test]
fn every_key_is_described() {
    let schema = json_schema();
    for subschema in subschemas(&schema) {
        let Some(properties) = subschema.get("properties").and_then(Value::as_object) else {
            continue;
        };
        for (key, property) in properties {
            // The contract `type` is described by the contract it selects.
            if key != "type" {
                assert!(property.get("description").is_some(), "`{key}` has no description");
            }
        }
    }
}

#[test]
fn the_schema_has_no_null() {
    // TOML has no null: an optional key is left out instead.
    let schema = json_schema().to_string();
    assert!(!schema.contains("null"), "{schema}");
}
