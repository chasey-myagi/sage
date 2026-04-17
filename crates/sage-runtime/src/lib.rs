//! Core agent execution engine for Sage.
//!
//! This crate provides:
//! - [`SageEngine`] / [`SageEngineBuilder`] — build and run an agent
//! - [`SageSession`] — multi-turn stateful conversation sessions
//! - [`AgentEvent`] / [`AgentEventSink`] — observe agent execution in real time
//! - LLM providers (Anthropic, OpenAI, Google, Bedrock)
//! - Built-in tools (bash, read, write, ls, find, grep, …)
//! - Context compaction for long conversations

pub mod agent;
pub mod agent_loop;
pub mod compaction;
pub mod engine;
pub mod event;
pub mod hook;
pub mod llm;
pub mod system_prompt;
pub mod tools;
pub mod types;

// Top-level re-exports for convenience
pub use agent::{StopAction, StopContext, StopHook, TransformContextHook};
pub use agent_loop::AgentLoopError;
pub use compaction::ContextBudget;
pub use engine::{SageEngine, SageEngineBuilder, SageError, SageSession, SandboxSettings};
pub use event::{AgentEvent, EventReceiver};
pub use hook::{HookBus, HookEvent, HookHandler, HookOutcome, HookReceiver};
pub use system_prompt::{PromptSection, SystemPrompt, SystemPromptBuilder};
pub use tools::AgentTool as SageTool;

#[cfg(test)]
pub mod test_helpers;
