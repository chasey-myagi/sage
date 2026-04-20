//! Ls (directory listing) tool definition and metadata.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/tools/ls.ts`.

/// Name of the ls tool as sent to the LLM API.
pub const TOOL_NAME: &str = "ls";

/// Default maximum number of directory entries returned by the ls tool.
pub const DEFAULT_LIMIT: usize = 500;

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
        let desc = descs.get(&ToolName::Ls).expect("ls descriptor must exist");
        assert_eq!(desc.name.to_string(), TOOL_NAME);
    }

    #[test]
    fn ls_descriptor_is_not_mutating() {
        let descs = all_tool_descriptors();
        assert!(!descs[&ToolName::Ls].mutating);
    }

    #[test]
    fn ls_description_mentions_dotfiles() {
        let descs = all_tool_descriptors();
        let desc = descs[&ToolName::Ls].description;
        assert!(desc.contains("dotfiles") || desc.contains("dot"));
    }

    #[test]
    fn ls_schema_has_no_required_fields() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Ls].parameters;
        let required = schema["required"].as_array().unwrap();
        assert!(required.is_empty());
    }

    #[test]
    fn ls_schema_has_optional_limit() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Ls].parameters;
        assert!(schema["properties"]["limit"].is_object());
    }

    #[test]
    fn default_limit_is_500() {
        assert_eq!(DEFAULT_LIMIT, 500);
    }
}
