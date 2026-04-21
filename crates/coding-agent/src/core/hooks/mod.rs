//! Hooks system — PreToolUse / PostToolUse / Stop external command hooks.
//!
//! Configuration is loaded from the `hooks` key in `settings.json`.
//! Hooks execute arbitrary shell commands at lifecycle events.
//!
//! Translated from CC `src/utils/hooks.ts` and `src/schemas/hooks.ts`.

pub mod executor;
pub mod runner;
pub mod types;

// Re-export the settings type used by settings_manager.
pub use types::HooksSettings;
