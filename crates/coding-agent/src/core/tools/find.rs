//! Find tool definition and metadata.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/tools/find.ts`.

/// Name of the find tool as sent to the LLM API.
pub const TOOL_NAME: &str = "find";

/// Default maximum number of results returned by the find tool.
pub const DEFAULT_LIMIT: usize = 1000;

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
            .get(&ToolName::Find)
            .expect("find descriptor must exist");
        assert_eq!(desc.name.to_string(), TOOL_NAME);
    }

    #[test]
    fn find_descriptor_is_not_mutating() {
        let descs = all_tool_descriptors();
        assert!(!descs[&ToolName::Find].mutating);
    }

    #[test]
    fn find_description_mentions_glob() {
        let descs = all_tool_descriptors();
        let desc = descs[&ToolName::Find].description;
        assert!(desc.contains("glob") || desc.contains("pattern"));
    }

    #[test]
    fn find_schema_requires_pattern() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Find].parameters;
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("pattern")));
    }

    #[test]
    fn find_schema_has_optional_limit() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Find].parameters;
        assert!(schema["properties"]["limit"].is_object());
    }

    #[test]
    fn default_limit_is_1000() {
        assert_eq!(DEFAULT_LIMIT, 1000);
    }
}
