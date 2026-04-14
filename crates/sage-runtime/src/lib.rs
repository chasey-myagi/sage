pub mod agent;
pub mod agent_loop;
pub mod compaction;
pub mod engine;
pub mod event;
pub mod llm;
pub mod system_prompt;
pub mod tools;
pub mod types;

// Top-level re-exports for convenience
pub use agent::TransformContextHook;
pub use compaction::ContextBudget;
pub use engine::{SageEngine, SageEngineBuilder, SageError, SandboxSettings};
pub use event::{AgentEvent, EventReceiver};
pub use system_prompt::{PromptSection, SystemPrompt, SystemPromptBuilder};
pub use tools::AgentTool as SageTool;

#[cfg(test)]
pub mod test_helpers;
