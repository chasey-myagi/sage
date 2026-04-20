//! Write tool definition and metadata.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/tools/write.ts`.

/// Name of the write tool as sent to the LLM API.
pub const TOOL_NAME: &str = "write";

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tools::{all_tool_descriptors, ToolName};

    #[test]
    fn tool_name_constant_matches_enum() {
        let descs = all_tool_descriptors();
        let desc = descs.get(&ToolName::Write).expect("write descriptor must exist");
        assert_eq!(desc.name.to_string(), TOOL_NAME);
    }

    #[test]
    fn write_descriptor_is_mutating() {
        let descs = all_tool_descriptors();
        assert!(descs[&ToolName::Write].mutating);
    }

    #[test]
    fn write_description_mentions_overwrite() {
        let descs = all_tool_descriptors();
        let desc = descs[&ToolName::Write].description;
        assert!(desc.contains("overwrites") || desc.contains("overwrite") || desc.contains("Creates"));
    }

    #[test]
    fn write_schema_requires_path_and_content() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Write].parameters;
        let required = schema["required"].as_array().unwrap();
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"path"));
        assert!(names.contains(&"content"));
    }
}
