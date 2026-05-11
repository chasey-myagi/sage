//! Tool-definition wrapper utilities.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/tools/tool-definition-wrapper.ts`.
//!
//! Provides helpers to adapt between the `ToolDescriptor` type used internally
//! by the coding agent and the `LlmTool` type consumed by the AI layer.

use ai::types::LlmTool;

// ============================================================================
// ToolWrapper
// ============================================================================

/// A lightweight runtime wrapper around a tool name, description, and schema.
///
/// Mirrors the shape that `wrapToolDefinition()` produces in TypeScript:
/// a plain object with `name`, `description`, and `parameters`.
#[derive(Debug, Clone)]
pub struct ToolWrapper {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolWrapper {
    /// Create a new `ToolWrapper` from constituent parts.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }

    /// Convert to an `LlmTool` suitable for the AI API layer.
    pub fn to_llm_tool(&self) -> LlmTool {
        LlmTool {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }
}

// ============================================================================
// wrap_tool_definition
// ============================================================================

/// Create a `ToolWrapper` from an `LlmTool`.
///
/// Mirrors `wrapToolDefinition()` from `tool-definition-wrapper.ts`.
pub fn wrap_tool_definition(tool: LlmTool) -> ToolWrapper {
    ToolWrapper {
        name: tool.name,
        description: tool.description,
        parameters: tool.parameters,
    }
}

/// Create `ToolWrapper`s from a list of `LlmTool`s.
///
/// Mirrors `wrapToolDefinitions()` from `tool-definition-wrapper.ts`.
pub fn wrap_tool_definitions(tools: Vec<LlmTool>) -> Vec<ToolWrapper> {
    tools.into_iter().map(wrap_tool_definition).collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_llm_tool() -> LlmTool {
        LlmTool {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "input": {"type": "string"}
                },
                "required": ["input"]
            }),
        }
    }

    // ---- ToolWrapper::new ----

    #[test]
    fn tool_wrapper_new_stores_fields() {
        let schema = serde_json::json!({"type": "object"});
        let wrapper = ToolWrapper::new("my_tool", "does stuff", schema.clone());
        assert_eq!(wrapper.name, "my_tool");
        assert_eq!(wrapper.description, "does stuff");
        assert_eq!(wrapper.parameters, schema);
    }

    // ---- ToolWrapper::to_llm_tool ----

    #[test]
    fn to_llm_tool_preserves_all_fields() {
        let schema = serde_json::json!({"type": "object", "properties": {}});
        let wrapper = ToolWrapper::new("bash", "Execute bash", schema.clone());
        let llm_tool = wrapper.to_llm_tool();
        assert_eq!(llm_tool.name, "bash");
        assert_eq!(llm_tool.description, "Execute bash");
        assert_eq!(llm_tool.parameters, schema);
    }

    // ---- wrap_tool_definition ----

    #[test]
    fn wrap_tool_definition_roundtrip() {
        let tool = sample_llm_tool();
        let wrapper = wrap_tool_definition(tool.clone());
        assert_eq!(wrapper.name, tool.name);
        assert_eq!(wrapper.description, tool.description);
        assert_eq!(wrapper.parameters, tool.parameters);
    }

    // ---- wrap_tool_definitions ----

    #[test]
    fn wrap_tool_definitions_preserves_order() {
        let tools = vec![
            LlmTool {
                name: "a".to_string(),
                description: "A".to_string(),
                parameters: serde_json::json!({}),
            },
            LlmTool {
                name: "b".to_string(),
                description: "B".to_string(),
                parameters: serde_json::json!({}),
            },
            LlmTool {
                name: "c".to_string(),
                description: "C".to_string(),
                parameters: serde_json::json!({}),
            },
        ];
        let wrappers = wrap_tool_definitions(tools);
        assert_eq!(wrappers.len(), 3);
        assert_eq!(wrappers[0].name, "a");
        assert_eq!(wrappers[1].name, "b");
        assert_eq!(wrappers[2].name, "c");
    }

    #[test]
    fn wrap_tool_definitions_empty() {
        let wrappers = wrap_tool_definitions(vec![]);
        assert!(wrappers.is_empty());
    }

    // ---- roundtrip to_llm_tool ----

    #[test]
    fn wrapped_to_llm_tool_is_identical_to_original() {
        let original = sample_llm_tool();
        let wrapper = wrap_tool_definition(original.clone());
        let converted = wrapper.to_llm_tool();
        assert_eq!(converted.name, original.name);
        assert_eq!(converted.description, original.description);
        assert_eq!(converted.parameters, original.parameters);
    }
}
