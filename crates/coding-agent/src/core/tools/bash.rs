//! Bash tool definition and execution metadata.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/tools/bash.ts`.
//!
//! This module re-exports the bash tool descriptor and provides the
//! tool-name constant used by the bash tool.  Actual execution is performed
//! by the agent runtime via `ToolBackend`.

/// Name of the bash tool as sent to the LLM API.
pub const TOOL_NAME: &str = "bash";

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
        let desc = descs.get(&ToolName::Bash).expect("bash descriptor must exist");
        assert_eq!(desc.name.to_string(), TOOL_NAME);
    }

    #[test]
    fn bash_descriptor_is_mutating() {
        let descs = all_tool_descriptors();
        assert!(descs[&ToolName::Bash].mutating);
    }

    #[test]
    fn bash_description_contains_key_terms() {
        let descs = all_tool_descriptors();
        let desc = descs[&ToolName::Bash].description;
        assert!(desc.contains("bash command"));
        assert!(desc.contains("timeout"));
    }

    #[test]
    fn bash_schema_required_command() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Bash].parameters;
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("command")));
    }

    #[test]
    fn bash_schema_optional_timeout() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::Bash].parameters;
        assert!(schema["properties"]["timeout"].is_object());
    }
}
