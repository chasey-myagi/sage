//! Tool wrappers for extension-registered tools.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/extensions/wrapper.ts`.
//!
//! Adapts extension `RegisteredTool` definitions so the agent session can
//! invoke them using a consistent context snapshot.

use serde_json::Value;

use super::runner::ExtensionRunner;
use super::types::RegisteredTool;

/// A wrapped tool ready for use by the agent.
#[derive(Debug)]
pub struct WrappedTool {
    /// Tool name.
    pub name: String,
    /// Tool description (for LLM).
    pub description: String,
    /// JSON Schema for parameters.
    pub parameters: Value,
    /// One-line snippet for the system prompt, if provided.
    pub prompt_snippet: Option<String>,
    /// Additional guideline bullets for the system prompt.
    pub prompt_guidelines: Vec<String>,
}

/// Wrap a single `RegisteredTool` into an agent-ready `WrappedTool`.
///
/// In TypeScript this creates an `AgentTool` that delegates execution through
/// `runner.createContext()`. In Rust we produce a lightweight descriptor;
/// actual execution is dispatched by the agent session.
pub fn wrap_registered_tool(registered_tool: &RegisteredTool) -> WrappedTool {
    let def = &registered_tool.definition;
    WrappedTool {
        name: def.name.clone(),
        description: def.description.clone(),
        parameters: def.parameters.clone(),
        prompt_snippet: def.prompt_snippet.clone(),
        prompt_guidelines: def.prompt_guidelines.clone(),
    }
}

/// Wrap all registered tools from the runner into `WrappedTool` descriptors.
pub fn wrap_registered_tools(runner: &ExtensionRunner) -> Vec<WrappedTool> {
    runner
        .get_all_registered_tools()
        .into_iter()
        .map(wrap_registered_tool)
        .collect()
}
