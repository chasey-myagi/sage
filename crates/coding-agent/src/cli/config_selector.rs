//! Config selector CLI command.
//!
//! Translated from pi-mono `packages/coding-agent/src/cli/config-selector.ts`.
//!
//! In TypeScript, this shows a TUI config selector. In Rust we provide a
//! stub that prints available config locations and exits, since the TUI
//! subsystem is not yet fully ported.

use std::path::PathBuf;

/// Options for the config selector.
#[derive(Debug, Clone)]
pub struct ConfigSelectorOptions {
    pub cwd: PathBuf,
    pub agent_dir: PathBuf,
}

/// Show config information (stub — no TUI yet).
///
/// Mirrors `selectConfig()` from TypeScript.
pub fn select_config(options: ConfigSelectorOptions) {
    println!("Configuration locations:");
    println!(
        "  Project config: {}",
        options.cwd.join(".pi").display()
    );
    println!("  Global config:  {}", options.agent_dir.display());
    println!();
    println!("Edit these directories to configure extensions, themes, skills, and prompts.");
}
