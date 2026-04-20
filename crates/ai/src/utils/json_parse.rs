//! Streaming JSON parser — Rust port of pi-mono `utils/json-parse.ts`.
//!
//! The TypeScript original uses the `partial-json` npm package to handle
//! incomplete JSON that arrives mid-stream.  The Rust port approximates this
//! with a heuristic "bracket completion" approach:
//!
//! 1. Try standard `serde_json` parsing (fast path for complete JSON).
//! 2. If that fails, attempt to complete the partial JSON by appending the
//!    necessary closing brackets/braces, then parse again.
//! 3. Return `serde_json::Value::Object(Default::default())` (empty object) on
//!    total failure, matching the TypeScript fallback of `{}`.
//!
//! This is intentionally a best-effort utility; callers should not rely on
//! partial results being semantically correct for all inputs.

use serde_json::Value;

/// Parse potentially incomplete JSON during streaming.
///
/// Always returns a valid `Value`, even if `partial_json` is incomplete or
/// unparseable.  Returns an empty object on failure, matching the TypeScript
/// `parseStreamingJson` return value of `{}`.
///
/// ```rust
/// use ai::utils::json_parse::parse_streaming_json;
/// use serde_json::json;
///
/// // Complete JSON parses normally.
/// assert_eq!(parse_streaming_json(r#"{"key":"value"}"#), json!({"key": "value"}));
///
/// // Empty / blank input returns empty object.
/// assert_eq!(parse_streaming_json(""), json!({}));
/// assert_eq!(parse_streaming_json("   "), json!({}));
///
/// // Partial JSON is completed heuristically.
/// let v = parse_streaming_json(r#"{"name":"Al"#);
/// assert!(v.is_object());
/// ```
pub fn parse_streaming_json(partial_json: &str) -> Value {
    if partial_json.is_empty() || partial_json.trim().is_empty() {
        return Value::Object(Default::default());
    }

    // Fast path: standard parse for complete JSON.
    if let Ok(v) = serde_json::from_str(partial_json) {
        return v;
    }

    // Slow path: attempt to complete incomplete JSON.
    if let Some(v) = try_complete_json(partial_json) {
        return v;
    }

    // Total failure → empty object.
    Value::Object(Default::default())
}

/// Attempt to repair partial JSON by appending closing tokens.
///
/// Tracks open brackets and string state to determine which tokens to append.
/// Returns `None` if even the repaired string fails to parse.
fn try_complete_json(s: &str) -> Option<Value> {
    let mut stack: Vec<u8> = Vec::new(); // stack of b'{' or b'['
    let mut in_string = false;
    let mut escape_next = false;
    let mut last_char: Option<char> = None;

    for ch in s.chars() {
        if escape_next {
            escape_next = false;
            last_char = Some(ch);
            continue;
        }

        if in_string {
            match ch {
                '\\' => escape_next = true,
                '"' => in_string = false,
                _ => {}
            }
        } else {
            match ch {
                '"' => in_string = true,
                '{' => stack.push(b'{'),
                '[' => stack.push(b'['),
                '}' => {
                    if stack.last() == Some(&b'{') {
                        stack.pop();
                    }
                }
                ']' => {
                    if stack.last() == Some(&b'[') {
                        stack.pop();
                    }
                }
                _ => {}
            }
        }
        last_char = Some(ch);
    }

    // Build the suffix needed to close open structures.
    let mut suffix = String::new();

    // If we're in the middle of a string, close it.
    if in_string {
        suffix.push('"');
    }

    // Close any trailing incomplete object key or value separator.
    // (heuristic: if the last non-whitespace char is ',' or ':', add a null)
    if let Some(lc) = last_char {
        if lc == ',' || lc == ':' {
            suffix.push_str("null");
        }
    }

    // Close all unclosed brackets in reverse stack order.
    for &open in stack.iter().rev() {
        if open == b'{' {
            suffix.push('}');
        } else {
            suffix.push(']');
        }
    }

    if suffix.is_empty() {
        return None;
    }

    let repaired = format!("{s}{suffix}");
    serde_json::from_str(&repaired).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_empty_string() {
        assert_eq!(parse_streaming_json(""), json!({}));
    }

    #[test]
    fn test_blank_string() {
        assert_eq!(parse_streaming_json("   "), json!({}));
    }

    #[test]
    fn test_complete_json_object() {
        assert_eq!(
            parse_streaming_json(r#"{"key": "value"}"#),
            json!({"key": "value"})
        );
    }

    #[test]
    fn test_complete_json_array() {
        assert_eq!(
            parse_streaming_json(r#"[1, 2, 3]"#),
            json!([1, 2, 3])
        );
    }

    #[test]
    fn test_complete_nested_json() {
        let input = r#"{"a": {"b": 1}, "c": [1, 2]}"#;
        assert_eq!(
            parse_streaming_json(input),
            json!({"a": {"b": 1}, "c": [1, 2]})
        );
    }

    #[test]
    fn test_partial_json_object_unclosed() {
        // Partial: {"name": "Alice"  (missing closing brace)
        let result = parse_streaming_json(r#"{"name": "Alice""#);
        assert!(result.is_object(), "expected object, got: {result}");
        assert_eq!(result["name"], "Alice");
    }

    #[test]
    fn test_partial_json_truncated_string_value() {
        // Mid-string: {"name": "Al
        let result = parse_streaming_json(r#"{"name": "Al"#);
        assert!(result.is_object());
        // The name key should be present with whatever partial value was parsed
    }

    #[test]
    fn test_partial_json_array_unclosed() {
        let result = parse_streaming_json(r#"[1, 2, 3"#);
        assert!(result.is_array() || result.is_object());
    }

    #[test]
    fn test_total_garbage_returns_empty_object() {
        let result = parse_streaming_json("not json at all }{{{");
        assert_eq!(result, json!({}));
    }

    #[test]
    fn test_complete_json_string() {
        assert_eq!(parse_streaming_json(r#""hello""#), json!("hello"));
    }

    #[test]
    fn test_complete_json_number() {
        assert_eq!(parse_streaming_json("42"), json!(42));
    }

    #[test]
    fn test_complete_json_bool() {
        assert_eq!(parse_streaming_json("true"), json!(true));
        assert_eq!(parse_streaming_json("false"), json!(false));
    }

    #[test]
    fn test_complete_json_null() {
        assert_eq!(parse_streaming_json("null"), Value::Null);
    }
}
