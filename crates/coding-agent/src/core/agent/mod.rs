//! Sub-agent system — Agent definition, fork mechanism, and execution engine.
//!
//! Mirrors CC `tools/AgentTool/` and related utilities.

pub mod builtin;
pub mod definition;
pub mod forked;
pub mod query_loop;
pub mod runner;

#[allow(unused_imports)]
pub use definition::{
    AgentDef, AgentLoader, AgentModel, AgentSource, MCPServerSpec, PermissionMode,
};
#[allow(unused_imports)]
pub use forked::{
    CacheSafeParams, FORK_BOILERPLATE_TAG, FORK_DIRECTIVE_PREFIX, FORK_PLACEHOLDER_RESULT,
    build_fork_boilerplate_message, build_forked_messages, filter_incomplete_tool_calls,
    is_in_fork_child,
};
#[allow(unused_imports)]
pub use runner::{AgentError, AgentRunResult, RunAgentParams, run_agent};
