//! Permission system for the coding agent.
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/permissions/`.
//!
//! Implements rule parsing, loading, and the core permission decision engine
//! with support for allow/deny/ask behaviors and multiple permission modes.

pub mod engine;
pub mod loader;
pub mod mode;
pub mod parser;

// Re-export most-used types for convenience.
pub use engine::{PermissionDecision, PermissionRuleSource, ToolPermissionContext};
pub use mode::PermissionMode;
pub use parser::PermissionBehavior;
