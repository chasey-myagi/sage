//! Run modes for the coding agent.
//!
//! Translated from pi-mono `packages/coding-agent/src/modes/`.
//!
//! Three modes exist:
//! - `print_mode`: single-shot (non-interactive) text or JSON output
//! - `interactive`: full TUI session
//! - `rpc`: JSON-RPC 2.0 server on stdin/stdout

pub mod interactive;
pub mod print_mode;
pub mod rpc;
