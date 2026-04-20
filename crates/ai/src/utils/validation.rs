//! Tool-argument validation — port of `utils/validation.ts`.
//!
//! pi-mono validates tool-call arguments against the tool's TypeBox JSON-Schema
//! using AJV.  sage doesn't pull in a full JSON-Schema validator (that's a
//! heavy dependency tree), so this module provides a **structural validation**
//! covering the cases the LLM actually breaks in practice:
//!
//! - Tool exists by name
//! - Arguments are a JSON object (when schema says `type: "object"`)
//! - All `required` properties are present
//! - Properties with `type` annotations have matching concrete types
//!
//! It deliberately skips AJV's edge cases (coercion, format checks, allOf /
//! oneOf fan-out) because those would bloat the Rust crate and are easy to
//! add later with a dedicated dependency. The *interface* mirrors pi-mono so
//! callers can be migrated without API changes.

use crate::types::LlmTool;
use serde_json::Value;

/// A tool call as seen by the validator.
#[derive(Debug, Clone)]
pub struct ToolCallView<'a> {
    pub name: &'a str,
    pub arguments: &'a Value,
}

/// Validation outcome — mirrors TypeScript's "return args on success, throw on failure".
pub type ValidateResult<'a> = Result<&'a Value, String>;

/// Find a tool by name in the tool set and validate the call.
///
/// Returns the (unchanged) arguments on success. `Err(msg)` is formatted in the
/// same style as pi-mono's thrown message so upstream logs look identical.
pub fn validate_tool_call<'a>(tools: &[LlmTool], call: &'a ToolCallView<'a>) -> ValidateResult<'a> {
    let tool = tools
        .iter()
        .find(|t| t.name == call.name)
        .ok_or_else(|| format!(r#"Tool "{}" not found"#, call.name))?;
    validate_tool_arguments(tool, call)
}

/// Validate that `call.arguments` satisfies `tool.parameters` (a JSON-Schema).
pub fn validate_tool_arguments<'a>(
    tool: &LlmTool,
    call: &'a ToolCallView<'a>,
) -> ValidateResult<'a> {
    let mut errors = Vec::new();
    validate_against_schema(&tool.parameters, call.arguments, "", &mut errors);

    if errors.is_empty() {
        return Ok(call.arguments);
    }

    // Mimic pi-mono's error block: one line per issue + echo of the arguments.
    let bullets = errors
        .iter()
        .map(|(path, msg)| {
            let p = if path.is_empty() {
                "root"
            } else {
                path.as_str()
            };
            format!("  - {p}: {msg}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let pretty_args =
        serde_json::to_string_pretty(call.arguments).unwrap_or_else(|_| call.arguments.to_string());
    Err(format!(
        "Validation failed for tool \"{name}\":\n{bullets}\n\nReceived arguments:\n{pretty_args}",
        name = call.name,
    ))
}

pub(crate) fn validate_against_schema(
    schema: &Value,
    value: &Value,
    path: &str,
    errors: &mut Vec<(String, String)>,
) {
    // Handle `type`
    if let Some(ty) = schema.get("type").and_then(Value::as_str) {
        if !type_matches(ty, value) {
            errors.push((
                path.to_string(),
                format!("must be {ty}, got {}", value_type_name(value)),
            ));
            // Don't dig into mismatched types — downstream checks would just
            // cascade noise.
            return;
        }
    }

    // Enum
    if let Some(variants) = schema.get("enum").and_then(Value::as_array) {
        if !variants.iter().any(|v| v == value) {
            errors.push((
                path.to_string(),
                format!(
                    "must be one of {}",
                    variants
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
    }

    // Object-specific checks
    if let Some(obj) = value.as_object() {
        // Required
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for req in required {
                if let Some(name) = req.as_str() {
                    if !obj.contains_key(name) {
                        errors.push((name.to_string(), "must have required property".into()));
                    }
                }
            }
        }

        // Properties
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (key, sub_schema) in props {
                if let Some(sub_value) = obj.get(key) {
                    let sub_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}/{key}")
                    };
                    validate_against_schema(sub_schema, sub_value, &sub_path, errors);
                }
            }
        }
    }

    // Array-specific checks
    if let (Some(items_schema), Some(arr)) = (schema.get("items"), value.as_array()) {
        for (idx, item) in arr.iter().enumerate() {
            let sub_path = if path.is_empty() {
                format!("[{idx}]")
            } else {
                format!("{path}[{idx}]")
            };
            validate_against_schema(items_schema, item, &sub_path, errors);
        }
    }
}

fn type_matches(expected: &str, value: &Value) -> bool {
    match expected {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => true, // unknown type — skip check
    }
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_f64() => "number",
        Value::Number(_) => "integer",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mktool(params: Value) -> LlmTool {
        LlmTool {
            name: "bash".into(),
            description: "run a command".into(),
            parameters: params,
        }
    }

    #[test]
    fn tool_not_found_returns_err() {
        let tools = vec![mktool(json!({"type": "object"}))];
        let args = json!({"x": 1});
        let call = ToolCallView {
            name: "nope",
            arguments: &args,
        };
        let err = validate_tool_call(&tools, &call).unwrap_err();
        assert!(err.contains("\"nope\""));
        assert!(err.contains("not found"));
    }

    #[test]
    fn ok_when_required_present() {
        let tool = mktool(json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"}
            },
            "required": ["command"]
        }));
        let args = json!({"command": "ls -la"});
        let call = ToolCallView {
            name: "bash",
            arguments: &args,
        };
        let out = validate_tool_call(&[tool], &call).unwrap();
        assert_eq!(out, &args);
    }

    #[test]
    fn err_when_required_missing() {
        let tool = mktool(json!({
            "type": "object",
            "properties": {"command": {"type": "string"}},
            "required": ["command"]
        }));
        let args = json!({});
        let call = ToolCallView {
            name: "bash",
            arguments: &args,
        };
        let err = validate_tool_call(&[tool], &call).unwrap_err();
        assert!(err.contains("command"));
        assert!(err.contains("required"));
        assert!(err.contains("Received arguments"));
    }

    #[test]
    fn err_when_type_mismatches() {
        let tool = mktool(json!({
            "type": "object",
            "properties": {"count": {"type": "integer"}},
            "required": ["count"]
        }));
        let args = json!({"count": "nope"});
        let call = ToolCallView {
            name: "bash",
            arguments: &args,
        };
        let err = validate_tool_call(&[tool], &call).unwrap_err();
        assert!(err.contains("count"));
        assert!(err.contains("integer"));
    }

    #[test]
    fn enum_enforced() {
        let tool = mktool(json!({
            "type": "object",
            "properties": {
                "mode": {"type": "string", "enum": ["read", "write"]}
            },
            "required": ["mode"]
        }));
        let bad = json!({"mode": "delete"});
        let call = ToolCallView {
            name: "bash",
            arguments: &bad,
        };
        assert!(validate_tool_call(&[tool.clone()], &call).is_err());

        let ok = json!({"mode": "read"});
        let call = ToolCallView {
            name: "bash",
            arguments: &ok,
        };
        assert!(validate_tool_call(&[tool], &call).is_ok());
    }

    #[test]
    fn nested_object_and_array() {
        let tool = mktool(json!({
            "type": "object",
            "properties": {
                "files": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {"path": {"type": "string"}},
                        "required": ["path"]
                    }
                }
            },
            "required": ["files"]
        }));
        // Second element missing `path` — should fail.
        let bad = json!({"files": [{"path": "/tmp/a"}, {}]});
        let call = ToolCallView {
            name: "bash",
            arguments: &bad,
        };
        assert!(validate_tool_call(&[tool.clone()], &call).is_err());

        let good = json!({"files": [{"path": "/tmp/a"}, {"path": "/tmp/b"}]});
        let call = ToolCallView {
            name: "bash",
            arguments: &good,
        };
        assert!(validate_tool_call(&[tool], &call).is_ok());
    }

    #[test]
    fn unknown_type_is_permissive() {
        // Schemas referencing types we don't model (e.g. `"integer"` with
        // format="int64") must not spuriously fail.
        let tool = mktool(json!({
            "type": "object",
            "properties": {"x": {"type": "integer", "format": "int64"}},
            "required": ["x"]
        }));
        let args = json!({"x": 42});
        let call = ToolCallView {
            name: "bash",
            arguments: &args,
        };
        assert!(validate_tool_call(&[tool], &call).is_ok());
    }

    #[test]
    fn schema_without_type_accepts_anything() {
        // If schema has no "type" field, all values pass. Mirrors pi-mono's
        // AJV behavior when the schema is untyped.
        let tool = mktool(json!({
            "properties": {"x": {}}
        }));
        let args = json!({"x": 42});
        let call = ToolCallView {
            name: "bash",
            arguments: &args,
        };
        assert!(validate_tool_call(&[tool], &call).is_ok());
    }

    #[test]
    fn null_value_passes_null_schema() {
        let tool = mktool(json!({
            "type": "object",
            "properties": {"v": {"type": "null"}},
            "required": ["v"]
        }));
        let args = json!({"v": null});
        let call = ToolCallView {
            name: "bash",
            arguments: &args,
        };
        assert!(validate_tool_call(&[tool], &call).is_ok());
    }

    #[test]
    fn boolean_type_enforced() {
        let tool = mktool(json!({
            "type": "object",
            "properties": {"flag": {"type": "boolean"}},
            "required": ["flag"]
        }));
        let good = json!({"flag": true});
        let call = ToolCallView {
            name: "bash",
            arguments: &good,
        };
        assert!(validate_tool_call(&[tool.clone()], &call).is_ok());

        let bad = json!({"flag": "yes"});
        let call = ToolCallView {
            name: "bash",
            arguments: &bad,
        };
        assert!(validate_tool_call(&[tool], &call).is_err());
    }

    #[test]
    fn validate_tool_arguments_returns_same_args_on_success() {
        // Mirrors pi-mono's "return args on success" contract.
        let tool = mktool(json!({
            "type": "object",
            "properties": {"n": {"type": "number"}},
            "required": ["n"]
        }));
        let args = json!({"n": 3.14});
        let call = ToolCallView {
            name: "bash",
            arguments: &args,
        };
        let result = validate_tool_arguments(&tool, &call).unwrap();
        assert_eq!(result, &args, "must return the same arg value on success");
    }

    #[test]
    fn multiple_required_missing_errors_all_reported() {
        let tool = mktool(json!({
            "type": "object",
            "properties": {
                "a": {"type": "string"},
                "b": {"type": "string"},
                "c": {"type": "string"}
            },
            "required": ["a", "b", "c"]
        }));
        let args = json!({});
        let call = ToolCallView {
            name: "bash",
            arguments: &args,
        };
        let err = validate_tool_call(&[tool], &call).unwrap_err();
        // All three missing properties should appear in the error.
        assert!(err.contains("a"), "error should mention 'a': {err}");
        assert!(err.contains("b"), "error should mention 'b': {err}");
        assert!(err.contains("c"), "error should mention 'c': {err}");
    }

    #[test]
    fn array_type_enforced() {
        let tool = mktool(json!({
            "type": "object",
            "properties": {"tags": {"type": "array"}},
            "required": ["tags"]
        }));
        let good = json!({"tags": ["a", "b"]});
        let call = ToolCallView {
            name: "bash",
            arguments: &good,
        };
        assert!(validate_tool_call(&[tool.clone()], &call).is_ok());

        let bad = json!({"tags": "not-an-array"});
        let call = ToolCallView {
            name: "bash",
            arguments: &bad,
        };
        assert!(validate_tool_call(&[tool], &call).is_err());
    }

    #[test]
    fn error_message_contains_tool_name_and_args() {
        // pi-mono's Validation failed block includes tool name + JSON args.
        let tool = mktool(json!({
            "type": "object",
            "properties": {"x": {"type": "integer"}},
            "required": ["x"]
        }));
        let args = json!({"x": "wrong"});
        let call = ToolCallView {
            name: "bash",
            arguments: &args,
        };
        let err = validate_tool_call(&[tool], &call).unwrap_err();
        assert!(err.contains("bash"), "error must contain tool name");
        assert!(err.contains("Received arguments"), "error must echo args");
    }
}
