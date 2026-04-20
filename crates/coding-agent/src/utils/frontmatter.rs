//! YAML frontmatter parsing.
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/frontmatter.ts`.
//!
//! A frontmatter block is opened and closed by `---` lines at the top of a
//! text file. Everything between the delimiters is parsed as YAML; everything
//! after the closing delimiter is returned as the body (with surrounding
//! whitespace trimmed, matching the reference implementation).

use serde::Deserialize;

/// Result of parsing a file with optional YAML frontmatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFrontmatter {
    /// Raw YAML as a `serde_json::Value`. Empty object if there was no
    /// frontmatter block.
    pub frontmatter: serde_json::Value,
    /// The body after the closing `---` (trimmed).
    pub body: String,
}

fn normalize_newlines(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

/// Split a file into its raw YAML frontmatter string and the body.
fn extract_frontmatter(content: &str) -> (Option<String>, String) {
    let normalized = normalize_newlines(content);

    if !normalized.starts_with("---") {
        return (None, normalized);
    }

    // Find closing `\n---` starting from index 3 (just after the opening `---`).
    // The reference looks for `indexOf("\n---", 3)`.
    let Some(end_rel) = normalized[3..].find("\n---") else {
        return (None, normalized);
    };
    let end_idx = end_rel + 3;

    // The TS slice is `normalized.slice(4, endIndex)` which skips the
    // opening `---\n` (4 chars) up to (but not including) the newline that
    // begins the closing `\n---`.
    // Body: `normalized.slice(endIndex + 4).trim()` — skip the `\n---` and trim.
    let yaml = normalized[4..end_idx].to_string();
    let body_start = end_idx + 4;
    let body = if body_start >= normalized.len() {
        String::new()
    } else {
        normalized[body_start..].trim().to_string()
    };
    (Some(yaml), body)
}

/// Parse YAML frontmatter from `content`.
///
/// Returns the parsed frontmatter as a `serde_json::Value` (object, or
/// empty object if there was no frontmatter block or it was null) along
/// with the body.
pub fn parse_frontmatter(content: &str) -> ParsedFrontmatter {
    let (yaml_str, body) = extract_frontmatter(content);
    let frontmatter = match yaml_str {
        None => serde_json::Value::Object(Default::default()),
        Some(y) => match serde_yaml::from_str::<serde_yaml::Value>(&y) {
            Ok(serde_yaml::Value::Null) | Err(_) => {
                serde_json::Value::Object(Default::default())
            }
            Ok(v) => yaml_to_json(v),
        },
    };
    ParsedFrontmatter { frontmatter, body }
}

/// Parse frontmatter into a strongly-typed struct via `serde`. Returns the
/// strongly-typed frontmatter and the body.
///
/// If there is no frontmatter block, `T::default()` is used.
pub fn parse_frontmatter_as<T>(content: &str) -> (T, String)
where
    T: for<'de> Deserialize<'de> + Default,
{
    let (yaml_str, body) = extract_frontmatter(content);
    let fm = match yaml_str {
        None => T::default(),
        Some(y) => serde_yaml::from_str::<T>(&y).unwrap_or_default(),
    };
    (fm, body)
}

/// Strip the frontmatter block from `content`, returning only the body.
pub fn strip_frontmatter(content: &str) -> String {
    parse_frontmatter(content).body
}

fn yaml_to_json(value: serde_yaml::Value) -> serde_json::Value {
    match value {
        serde_yaml::Value::Null => serde_json::Value::Null,
        serde_yaml::Value::Bool(b) => serde_json::Value::Bool(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::Value::Number(i.into())
            } else if let Some(u) = n.as_u64() {
                serde_json::Value::Number(u.into())
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)
            } else {
                serde_json::Value::Null
            }
        }
        serde_yaml::Value::String(s) => serde_json::Value::String(s),
        serde_yaml::Value::Sequence(seq) => {
            serde_json::Value::Array(seq.into_iter().map(yaml_to_json).collect())
        }
        serde_yaml::Value::Mapping(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let key = match k {
                    serde_yaml::Value::String(s) => s,
                    other => match serde_yaml::to_string(&other) {
                        Ok(s) => s.trim().to_string(),
                        Err(_) => continue,
                    },
                };
                out.insert(key, yaml_to_json(v));
            }
            serde_json::Value::Object(out)
        }
        serde_yaml::Value::Tagged(t) => yaml_to_json(t.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_frontmatter_returns_body() {
        let content = "Just body text.";
        let result = parse_frontmatter(content);
        assert_eq!(result.frontmatter, serde_json::json!({}));
        assert_eq!(result.body, "Just body text.");
    }

    #[test]
    fn simple_frontmatter_and_body() {
        let input = "---\nname: test-skill\ndescription: Test description\n---\nBody text.\n";
        let result = parse_frontmatter(input);
        assert_eq!(result.frontmatter["name"], "test-skill");
        assert_eq!(result.frontmatter["description"], "Test description");
        assert_eq!(result.body, "Body text.");
    }

    #[test]
    fn crlf_line_endings_are_normalized() {
        let input = "---\r\nname: x\r\ndescription: d\r\n---\r\nBody.\r\n";
        let result = parse_frontmatter(input);
        assert_eq!(result.frontmatter["name"], "x");
        assert_eq!(result.body, "Body.");
    }

    #[test]
    fn unclosed_frontmatter_treated_as_body() {
        let input = "---\nno closing";
        let result = parse_frontmatter(input);
        // No closing `\n---` -> no frontmatter extracted.
        assert_eq!(result.frontmatter, serde_json::json!({}));
        assert_eq!(result.body, "---\nno closing");
    }

    #[test]
    fn trailing_whitespace_trimmed_from_body() {
        let input = "---\nname: x\ndescription: d\n---\n\n  body  \n\n";
        let result = parse_frontmatter(input);
        assert_eq!(result.body, "body");
    }

    #[test]
    fn boolean_frontmatter() {
        let input = "---\nname: a\ndescription: b\ndisable-model-invocation: true\n---\nbody";
        let result = parse_frontmatter(input);
        assert_eq!(result.frontmatter["disable-model-invocation"], true);
    }

    #[test]
    fn nested_mapping_frontmatter() {
        let input = "---\nmeta:\n  kind: test\n  count: 3\n---\nbody";
        let result = parse_frontmatter(input);
        assert_eq!(result.frontmatter["meta"]["kind"], "test");
        assert_eq!(result.frontmatter["meta"]["count"], 3);
    }

    #[test]
    fn strip_frontmatter_returns_body_only() {
        let input = "---\nname: x\ndescription: d\n---\njust body";
        assert_eq!(strip_frontmatter(input), "just body");
    }

    #[test]
    fn parse_frontmatter_as_typed() {
        #[derive(Debug, Default, Deserialize, PartialEq, Eq)]
        struct SkillFm {
            name: Option<String>,
            description: Option<String>,
            #[serde(rename = "disable-model-invocation", default)]
            disable_model_invocation: bool,
        }

        let input = "---\nname: x\ndescription: d\ndisable-model-invocation: true\n---\nbody";
        let (fm, body) = parse_frontmatter_as::<SkillFm>(input);
        assert_eq!(fm.name.as_deref(), Some("x"));
        assert_eq!(fm.description.as_deref(), Some("d"));
        assert!(fm.disable_model_invocation);
        assert_eq!(body, "body");
    }

    #[test]
    fn parse_frontmatter_as_missing_defaults() {
        #[derive(Debug, Default, Deserialize, PartialEq, Eq)]
        struct SkillFm {
            name: Option<String>,
        }
        let (fm, body) = parse_frontmatter_as::<SkillFm>("no frontmatter here");
        assert!(fm.name.is_none());
        assert_eq!(body, "no frontmatter here");
    }

    #[test]
    fn empty_frontmatter_block_is_empty_object() {
        // Empty frontmatter body — YAML parses to null, we normalize to {}.
        let input = "---\n\n---\nbody";
        let result = parse_frontmatter(input);
        assert_eq!(result.frontmatter, serde_json::json!({}));
        assert_eq!(result.body, "body");
    }
}
