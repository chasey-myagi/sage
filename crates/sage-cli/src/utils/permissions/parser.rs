//! Permission rule string parser.
//!
//! Translated from pi-mono `utils/permissions/permissionRuleParser.ts`.
//!
//! Handles parsing and serialization of rule strings in the format:
//! - `"ToolName"` — matches the entire tool
//! - `"ToolName(content)"` — matches tool with specific content
//!
//! Parentheses in content are escaped with backslashes.

use serde::{Deserialize, Serialize};

// ============================================================================
// Types
// ============================================================================

/// Whether a rule allows, denies, or asks for confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionBehavior {
    Allow,
    Deny,
    Ask,
}

impl std::fmt::Display for PermissionBehavior {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PermissionBehavior::Allow => write!(f, "allow"),
            PermissionBehavior::Deny => write!(f, "deny"),
            PermissionBehavior::Ask => write!(f, "ask"),
        }
    }
}

impl PermissionBehavior {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "allow" => Some(PermissionBehavior::Allow),
            "deny" => Some(PermissionBehavior::Deny),
            "ask" => Some(PermissionBehavior::Ask),
            _ => None,
        }
    }
}

/// The parsed content of a permission rule.
///
/// - `tool_name`: The name of the tool this rule applies to.
/// - `rule_content`: Optional content filter (e.g. a command prefix).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PermissionRuleValue {
    pub tool_name: String,
    pub rule_content: Option<String>,
}

// ============================================================================
// Legacy name normalization
// ============================================================================

/// Maps legacy tool names to their current canonical names.
const LEGACY_ALIASES: &[(&str, &str)] = &[
    ("Task", "Agent"),
    ("KillShell", "TaskStop"),
    ("AgentOutputTool", "TaskOutput"),
    ("BashOutputTool", "TaskOutput"),
];

/// Normalize a legacy tool name to its canonical form.
pub fn normalize_legacy_tool_name(name: &str) -> &str {
    for (legacy, canonical) in LEGACY_ALIASES {
        if *legacy == name {
            return canonical;
        }
    }
    name
}

/// Returns all legacy names that map to the given canonical tool name.
pub fn get_legacy_tool_names(canonical: &str) -> Vec<&'static str> {
    LEGACY_ALIASES
        .iter()
        .filter(|(_, c)| *c == canonical)
        .map(|(legacy, _)| *legacy)
        .collect()
}

// ============================================================================
// Escape / unescape
// ============================================================================

/// Escape special characters in rule content for safe storage.
///
/// Escaping order matters:
/// 1. Escape existing backslashes first (`\` → `\\`)
/// 2. Escape opening parentheses (`(` → `\(`)
/// 3. Escape closing parentheses (`)` → `\)`)
pub fn escape_rule_content(content: &str) -> String {
    content
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

/// Unescape special characters in rule content after parsing.
///
/// Unescaping order (reverse of escaping):
/// 1. Unescape opening parentheses (`\(` → `(`)
/// 2. Unescape closing parentheses (`\)` → `)`)
/// 3. Unescape backslashes (`\\` → `\`)
pub fn unescape_rule_content(content: &str) -> String {
    content
        .replace("\\(", "(")
        .replace("\\)", ")")
        .replace("\\\\", "\\")
}

// ============================================================================
// Character scanning helpers
// ============================================================================

/// Find the byte index of the first unescaped occurrence of `target` in `s`.
///
/// Scans bytes directly: `\X` pairs are skipped as escape sequences.
/// Safe for UTF-8 because `(` and `)` are single-byte ASCII values that
/// never appear inside multi-byte sequences.
fn find_first_unescaped(s: &str, target: char) -> Option<usize> {
    let target_byte = target as u8;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == target_byte {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Find the byte index of the last unescaped occurrence of `target` in `s`.
///
/// Scans bytes directly: `\X` pairs are skipped as escape sequences.
fn find_last_unescaped(s: &str, target: char) -> Option<usize> {
    let target_byte = target as u8;
    let bytes = s.as_bytes();
    let mut result = None;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == target_byte {
            result = Some(i);
        }
        i += 1;
    }
    result
}

// ============================================================================
// Public parsing API
// ============================================================================

/// Parse a permission rule string into its components.
///
/// Format: `"ToolName"` or `"ToolName(content)"`.
/// Content may contain escaped parentheses (`\(` and `\)`).
///
/// Falls back to treating the entire string as a tool name when the format
/// is malformed (no matching parens, empty tool name, etc.).
pub fn permission_rule_value_from_str(rule_str: &str) -> PermissionRuleValue {
    let fallback = || PermissionRuleValue {
        tool_name: normalize_legacy_tool_name(rule_str).to_owned(),
        rule_content: None,
    };

    let Some(open_byte) = find_first_unescaped(rule_str, '(') else {
        return fallback();
    };

    let Some(close_byte) = find_last_unescaped(rule_str, ')') else {
        return fallback();
    };

    if close_byte <= open_byte {
        return fallback();
    }

    // Closing paren must be the last character in the string.
    let last_char_byte = rule_str.char_indices().last().map(|(i, _)| i).unwrap_or(0);
    if close_byte != last_char_byte {
        return fallback();
    }

    let tool_name = &rule_str[..open_byte];
    if tool_name.is_empty() {
        return fallback();
    }

    // `open_byte` points to `(`, the content starts at `open_byte + 1`.
    let raw_content = &rule_str[open_byte + 1..close_byte];

    // Empty content or bare wildcard — treat as tool-wide rule.
    if raw_content.is_empty() || raw_content == "*" {
        return PermissionRuleValue {
            tool_name: normalize_legacy_tool_name(tool_name).to_owned(),
            rule_content: None,
        };
    }

    PermissionRuleValue {
        tool_name: normalize_legacy_tool_name(tool_name).to_owned(),
        rule_content: Some(unescape_rule_content(raw_content)),
    }
}

/// Serialize a `PermissionRuleValue` back to its string representation.
pub fn permission_rule_value_to_string(rv: &PermissionRuleValue) -> String {
    match &rv.rule_content {
        None => rv.tool_name.clone(),
        Some(content) => format!("{}({})", rv.tool_name, escape_rule_content(content)),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ─── escape / unescape ───────────────────────────────────────────────────

    #[test]
    fn escape_parens() {
        assert_eq!(
            escape_rule_content("psycopg2.connect()"),
            "psycopg2.connect\\(\\)"
        );
    }

    #[test]
    fn escape_backslash() {
        assert_eq!(
            escape_rule_content("echo \"test\\nvalue\""),
            "echo \"test\\\\nvalue\""
        );
    }

    #[test]
    fn unescape_parens() {
        assert_eq!(
            unescape_rule_content("psycopg2.connect\\(\\)"),
            "psycopg2.connect()"
        );
    }

    #[test]
    fn unescape_backslash() {
        assert_eq!(
            unescape_rule_content("echo \"test\\\\nvalue\""),
            "echo \"test\\nvalue\""
        );
    }

    // ─── from_str ────────────────────────────────────────────────────────────

    #[test]
    fn parse_bare_tool_name() {
        let rv = permission_rule_value_from_str("Bash");
        assert_eq!(rv.tool_name, "Bash");
        assert!(rv.rule_content.is_none());
    }

    #[test]
    fn parse_tool_with_content() {
        let rv = permission_rule_value_from_str("Bash(npm install)");
        assert_eq!(rv.tool_name, "Bash");
        assert_eq!(rv.rule_content.as_deref(), Some("npm install"));
    }

    #[test]
    fn parse_tool_with_escaped_parens_in_content() {
        let rv = permission_rule_value_from_str("Bash(python -c \"print\\(1\\)\")");
        assert_eq!(rv.tool_name, "Bash");
        assert_eq!(rv.rule_content.as_deref(), Some("python -c \"print(1)\""));
    }

    #[test]
    fn parse_empty_content_treated_as_no_content() {
        let rv = permission_rule_value_from_str("Bash()");
        assert_eq!(rv.tool_name, "Bash");
        assert!(rv.rule_content.is_none());
    }

    #[test]
    fn parse_wildcard_content_treated_as_no_content() {
        let rv = permission_rule_value_from_str("Bash(*)");
        assert_eq!(rv.tool_name, "Bash");
        assert!(rv.rule_content.is_none());
    }

    #[test]
    fn parse_empty_tool_name_falls_back() {
        let rv = permission_rule_value_from_str("(foo)");
        assert_eq!(rv.tool_name, "(foo)");
        assert!(rv.rule_content.is_none());
    }

    #[test]
    fn parse_content_after_close_paren_falls_back() {
        let rv = permission_rule_value_from_str("Bash(foo)extra");
        assert_eq!(rv.tool_name, "Bash(foo)extra");
        assert!(rv.rule_content.is_none());
    }

    // ─── legacy name normalization ───────────────────────────────────────────

    #[test]
    fn legacy_task_normalizes_to_agent() {
        let rv = permission_rule_value_from_str("Task");
        assert_eq!(rv.tool_name, "Agent");
    }

    #[test]
    fn legacy_kill_shell_normalizes_to_task_stop() {
        let rv = permission_rule_value_from_str("KillShell");
        assert_eq!(rv.tool_name, "TaskStop");
    }

    #[test]
    fn get_legacy_tool_names_for_task_output() {
        let legacy = get_legacy_tool_names("TaskOutput");
        assert!(legacy.contains(&"AgentOutputTool"));
        assert!(legacy.contains(&"BashOutputTool"));
    }

    // ─── to_string ───────────────────────────────────────────────────────────

    #[test]
    fn serialize_bare_tool_name() {
        let rv = PermissionRuleValue {
            tool_name: "Bash".to_string(),
            rule_content: None,
        };
        assert_eq!(permission_rule_value_to_string(&rv), "Bash");
    }

    #[test]
    fn serialize_with_content() {
        let rv = PermissionRuleValue {
            tool_name: "Bash".to_string(),
            rule_content: Some("npm install".to_string()),
        };
        assert_eq!(permission_rule_value_to_string(&rv), "Bash(npm install)");
    }

    #[test]
    fn serialize_with_parens_in_content() {
        let rv = PermissionRuleValue {
            tool_name: "Bash".to_string(),
            rule_content: Some("python -c \"print(1)\"".to_string()),
        };
        assert_eq!(
            permission_rule_value_to_string(&rv),
            "Bash(python -c \"print\\(1\\)\")"
        );
    }

    // ─── roundtrip ───────────────────────────────────────────────────────────

    #[test]
    fn roundtrip_bare_tool() {
        let original = "Bash";
        let rv = permission_rule_value_from_str(original);
        assert_eq!(permission_rule_value_to_string(&rv), original);
    }

    #[test]
    fn roundtrip_with_content() {
        let original = "Bash(npm install)";
        let rv = permission_rule_value_from_str(original);
        assert_eq!(permission_rule_value_to_string(&rv), original);
    }

    #[test]
    fn roundtrip_with_escaped_content() {
        let original = "Bash(python -c \"print\\(1\\)\")";
        let rv = permission_rule_value_from_str(original);
        assert_eq!(permission_rule_value_to_string(&rv), original);
    }
}
