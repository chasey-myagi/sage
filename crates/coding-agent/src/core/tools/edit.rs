//! Edit tool definition and metadata.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/tools/edit.ts`.

/// Name of the edit tool as sent to the LLM API.
pub const TOOL_NAME: &str = "edit";

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
            .get(&ToolName::Edit)
            .expect("edit descriptor must exist");
        assert_eq!(desc.name.to_string(), TOOL_NAME);
    }

    #[test]
    fn edit_descriptor_is_mutating() {
        let descs = all_tool_descriptors();
        assert!(descs[&ToolName::Edit].mutating);
    }

    #[test]
    fn edit_schema_has_three_required_fields() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Edit].parameters;
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 3);
    }

    #[test]
    fn edit_schema_has_old_text_and_new_text() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Edit].parameters;
        assert!(schema["properties"]["oldText"].is_object());
        assert!(schema["properties"]["newText"].is_object());
    }

    #[test]
    fn edit_description_mentions_exact_match() {
        let descs = all_tool_descriptors();
        let desc = descs[&ToolName::Edit].description;
        assert!(desc.contains("exact") || desc.contains("whitespace"));
    }
}
