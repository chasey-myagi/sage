//! Grep tool definition and metadata.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/tools/grep.ts`.

/// Name of the grep tool as sent to the LLM API.
pub const TOOL_NAME: &str = "grep";

/// Default maximum number of matches returned by the grep tool.
pub const DEFAULT_LIMIT: usize = 100;

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tools::{ToolName, all_tool_descriptors};

    #[test]
    fn tool_name_constant_matches_enum() {
        let descs = all_tool_descriptors();
        let desc = descs
            .get(&ToolName::Grep)
            .expect("grep descriptor must exist");
        assert_eq!(desc.name.to_string(), TOOL_NAME);
    }

    #[test]
    fn grep_descriptor_is_not_mutating() {
        let descs = all_tool_descriptors();
        assert!(!descs[&ToolName::Grep].mutating);
    }

    #[test]
    fn grep_description_mentions_gitignore() {
        let descs = all_tool_descriptors();
        let desc = descs[&ToolName::Grep].description;
        assert!(desc.contains(".gitignore"));
    }

    #[test]
    fn grep_schema_requires_pattern() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Grep].parameters;
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0].as_str(), Some("pattern"));
    }

    #[test]
    fn grep_schema_has_optional_params() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Grep].parameters;
        assert!(schema["properties"]["glob"].is_object());
        assert!(schema["properties"]["ignoreCase"].is_object());
        assert!(schema["properties"]["literal"].is_object());
        assert!(schema["properties"]["context"].is_object());
        assert!(schema["properties"]["limit"].is_object());
    }

    #[test]
    fn default_limit_is_100() {
        assert_eq!(DEFAULT_LIMIT, 100);
    }
}
