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
// These items are public API and may not be used within this crate itself.
#[allow(unused_imports)]
pub use engine::{
    PermissionDecision, PermissionDecisionReason, PermissionRule, PermissionRuleSource,
    ToolPermissionContext,
};
#[allow(unused_imports)]
pub use mode::PermissionMode;
#[allow(unused_imports)]
pub use parser::{PermissionBehavior, PermissionRuleValue};
