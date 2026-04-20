//! Read tool definition and metadata.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/tools/read.ts`.

/// Name of the read tool as sent to the LLM API.
pub const TOOL_NAME: &str = "read";

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
            .get(&ToolName::Read)
            .expect("read descriptor must exist");
        assert_eq!(desc.name.to_string(), TOOL_NAME);
    }

    #[test]
    fn read_descriptor_is_not_mutating() {
        let descs = all_tool_descriptors();
        assert!(!descs[&ToolName::Read].mutating);
    }

    #[test]
    fn read_description_mentions_images() {
        let descs = all_tool_descriptors();
        let desc = descs[&ToolName::Read].description;
        assert!(desc.contains("jpg") || desc.contains("png"));
    }

    #[test]
    fn read_schema_has_offset_and_limit() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Read].parameters;
        assert!(schema["properties"]["offset"].is_object());
        assert!(schema["properties"]["limit"].is_object());
    }

    #[test]
    fn read_schema_path_is_required() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Read].parameters;
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
    }
}
