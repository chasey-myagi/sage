//! JSON Schema construction helpers — Rust counterpart of `utils/typebox-helpers.ts`.
//!
//! pi-mono uses TypeBox (`Type.Object`, `Type.String`, …) to build JSON Schema
//! values at runtime, then validates them with AJV.  Here we build equivalent
//! `serde_json::Value` schemas using plain JSON and delegate validation to the
//! structural checker in [`crate::utils::validation`].
//!
//! All `*_schema` constructors produce standard JSON Schema draft-07 objects.

use serde_json::{Value, json};

use crate::utils::validation::validate_against_schema;

// ── Schema builders ──────────────────────────────────────────────────────────

/// `Type.Object({...})` — object schema with named properties and required list.
pub fn object_schema(properties: &[(&str, Value)], required: &[&str]) -> Value {
    let props: serde_json::Map<String, Value> = properties
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    json!({
        "type": "object",
        "properties": props,
        "required": required,
    })
}

/// `Type.String()` — plain string schema.
pub fn string_schema() -> Value {
    json!({ "type": "string" })
}

/// `StringEnum([...])` — string schema restricted to an enum of values.
///
/// Mirrors pi-mono's `StringEnum` helper which avoids `anyOf`/`const` patterns
/// incompatible with some providers (e.g. Google).
pub fn string_enum_schema(values: &[&str]) -> Value {
    json!({ "type": "string", "enum": values })
}

/// Options for [`string_enum_schema_with_options`].
pub struct StringEnumOptions<'a> {
    pub description: Option<&'a str>,
    pub default: Option<&'a str>,
}

/// `StringEnum([...], { description?, default? })` — string enum schema with optional metadata.
///
/// Extends [`string_enum_schema`] with the optional `description` and `default` fields
/// supported by the TypeScript counterpart.
pub fn string_enum_schema_with_options(values: &[&str], options: &StringEnumOptions) -> Value {
    let mut schema = json!({ "type": "string", "enum": values });
    if let Some(desc) = options.description {
        schema["description"] = json!(desc);
    }
    if let Some(default) = options.default {
        schema["default"] = json!(default);
    }
    schema
}

/// `Type.Number()` — number schema (integers and floats).
pub fn number_schema() -> Value {
    json!({ "type": "number" })
}

/// `Type.Boolean()` — boolean schema.
pub fn boolean_schema() -> Value {
    json!({ "type": "boolean" })
}

/// `Type.Array(items)` — array schema whose elements conform to `items`.
pub fn array_schema(items: Value) -> Value {
    json!({ "type": "array", "items": items })
}

/// `Type.Union([T, Type.Null()])` — nullable wrapper via `anyOf`.
///
/// Produces `{"anyOf": [<schema>, {"type": "null"}]}` which is understood by
/// all major validators.
pub fn nullable(schema: Value) -> Value {
    json!({ "anyOf": [schema, {"type": "null"}] })
}

// ── Validation ───────────────────────────────────────────────────────────────

/// Validate `value` against `schema`, returning human-readable error strings.
///
/// Delegates to the structural JSON Schema checker in [`validation`], which
/// covers `type`, `enum`, `required`, `properties`, and `items`.  Complex
/// keywords (`allOf`, `oneOf`, format checks) are intentionally not covered —
/// see `validation.rs` for rationale.
///
/// Returns `Ok(())` on success or `Err(errors)` where each string describes
/// one violation at its JSON Pointer path.
pub fn validate(schema: &Value, value: &Value) -> Result<(), Vec<String>> {
    let mut raw: Vec<(String, String)> = Vec::new();
    validate_against_schema(schema, value, "", &mut raw);

    if raw.is_empty() {
        return Ok(());
    }

    let msgs = raw
        .into_iter()
        .map(|(path, msg)| {
            let p = if path.is_empty() {
                "root".to_string()
            } else {
                path
            };
            format!("{p}: {msg}")
        })
        .collect();
    Err(msgs)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Schema builder shape tests ──────────────────────────────────────────

    #[test]
    fn object_schema_has_correct_type_and_fields() {
        let s = object_schema(&[("x", number_schema())], &["x"]);
        assert_eq!(s["type"], "object");
        assert!(s["properties"]["x"].is_object());
        assert_eq!(s["required"][0], "x");
    }

    #[test]
    fn string_schema_type_is_string() {
        assert_eq!(string_schema()["type"], "string");
    }

    #[test]
    fn string_enum_schema_carries_variants() {
        let s = string_enum_schema(&["add", "sub"]);
        assert_eq!(s["type"], "string");
        let variants = s["enum"].as_array().unwrap();
        assert_eq!(variants.len(), 2);
        assert!(variants.contains(&json!("add")));
        assert!(variants.contains(&json!("sub")));
    }

    #[test]
    fn string_enum_schema_with_options_no_extras() {
        let s = string_enum_schema_with_options(
            &["a", "b"],
            &StringEnumOptions {
                description: None,
                default: None,
            },
        );
        assert_eq!(s["type"], "string");
        assert!(s["description"].is_null());
        assert!(s["default"].is_null());
    }

    #[test]
    fn string_enum_schema_with_options_description() {
        let s = string_enum_schema_with_options(
            &["x", "y"],
            &StringEnumOptions {
                description: Some("pick one"),
                default: None,
            },
        );
        assert_eq!(s["description"], "pick one");
        assert!(s["default"].is_null());
    }

    #[test]
    fn string_enum_schema_with_options_default() {
        let s = string_enum_schema_with_options(
            &["x", "y"],
            &StringEnumOptions {
                description: None,
                default: Some("x"),
            },
        );
        assert!(s["description"].is_null());
        assert_eq!(s["default"], "x");
    }

    #[test]
    fn string_enum_schema_with_options_both() {
        let s = string_enum_schema_with_options(
            &["read", "write"],
            &StringEnumOptions {
                description: Some("access level"),
                default: Some("read"),
            },
        );
        assert_eq!(s["description"], "access level");
        assert_eq!(s["default"], "read");
        assert_eq!(s["type"], "string");
        let variants = s["enum"].as_array().unwrap();
        assert!(variants.contains(&json!("read")));
        assert!(variants.contains(&json!("write")));
    }

    #[test]
    fn number_schema_type_is_number() {
        assert_eq!(number_schema()["type"], "number");
    }

    #[test]
    fn boolean_schema_type_is_boolean() {
        assert_eq!(boolean_schema()["type"], "boolean");
    }

    #[test]
    fn array_schema_wraps_items() {
        let s = array_schema(string_schema());
        assert_eq!(s["type"], "array");
        assert_eq!(s["items"]["type"], "string");
    }

    #[test]
    fn nullable_produces_any_of_with_null() {
        let s = nullable(string_schema());
        let any_of = s["anyOf"].as_array().unwrap();
        assert_eq!(any_of.len(), 2);
        assert!(any_of.iter().any(|v| v["type"] == "null"));
        assert!(any_of.iter().any(|v| v["type"] == "string"));
    }

    // ── Validation tests ────────────────────────────────────────────────────

    #[test]
    fn validate_ok_on_matching_object() {
        let schema = object_schema(&[("name", string_schema())], &["name"]);
        let val = json!({"name": "alice"});
        assert!(validate(&schema, &val).is_ok());
    }

    #[test]
    fn validate_err_on_missing_required() {
        let schema = object_schema(&[("name", string_schema())], &["name"]);
        let val = json!({});
        let errs = validate(&schema, &val).unwrap_err();
        assert!(!errs.is_empty());
        assert!(
            errs[0].contains("name"),
            "error should mention field name: {errs:?}"
        );
    }

    #[test]
    fn validate_err_on_wrong_type() {
        let schema = object_schema(&[("count", number_schema())], &["count"]);
        let val = json!({"count": "oops"});
        let errs = validate(&schema, &val).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("number")),
            "errors: {errs:?}"
        );
    }

    #[test]
    fn validate_err_on_enum_violation() {
        let schema = object_schema(&[("op", string_enum_schema(&["read", "write"]))], &["op"]);
        let bad = json!({"op": "delete"});
        assert!(validate(&schema, &bad).is_err());

        let good = json!({"op": "read"});
        assert!(validate(&schema, &good).is_ok());
    }

    #[test]
    fn validate_array_items_recursively() {
        let schema = array_schema(string_schema());
        assert!(validate(&schema, &json!(["a", "b"])).is_ok());
        assert!(validate(&schema, &json!(["a", 99])).is_err());
    }

    #[test]
    fn validate_errors_include_path() {
        let schema = object_schema(&[("x", boolean_schema())], &["x"]);
        let val = json!({"x": "not-bool"});
        let errs = validate(&schema, &val).unwrap_err();
        // Path should appear — either "x:" or a slash-separated path.
        assert!(
            errs.iter().any(|e| e.contains('x')),
            "errors should mention field path: {errs:?}"
        );
    }
}
