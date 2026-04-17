//! Agent configuration, tool policy, hook execution, channel adapters, and metrics for Sage.
//!
//! Load an agent config with [`config::AgentConfig`], resolve tool permissions with
//! [`ToolPolicy`], run lifecycle hooks with [`hooks`], and observe outbound channels
//! via [`channel::ChannelAdapter`].

pub mod channel;
pub mod config;
pub mod hooks;
pub mod metrics;

pub use config::{AgentConfig, home_dir};
pub use sage_runtime::tools::policy::ToolPolicy;
