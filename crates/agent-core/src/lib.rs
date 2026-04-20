//! Agent execution framework.
//! Mirrors pi-mono's packages/agent (agent.ts, agent-loop.ts, proxy.ts, types.ts).

pub mod agent;
pub mod agent_loop;
pub mod bedrock_models;
pub mod compaction;
pub mod event;
pub mod hook;
pub mod proxy;
pub mod system_prompt;
pub mod tools;
pub mod transform;
pub mod types;

#[cfg(test)]
pub mod test_helpers;

pub use event::AgentEvent;
pub use types::*;
