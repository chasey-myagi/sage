//! Core modules for the coding agent.
//!
//! Mirrors the structure of pi-mono `packages/coding-agent/src/core/`.

pub mod agent;
pub mod agent_session;
pub mod auth_storage;
pub mod bash_executor;
pub mod compaction;
pub mod defaults;
pub mod diagnostics;
pub mod event_bus;
pub mod exec;
pub mod export_html;
pub mod extensions;
pub mod footer_data_provider;
pub mod keybindings;
pub mod messages;
pub mod model_registry;
pub mod model_resolver;
pub mod output_guard;
pub mod package_manager;
pub mod prompt_templates;
pub mod resolve_config_value;
pub mod resource_loader;
pub mod sdk;
pub mod session_manager;
pub mod settings_manager;
pub mod skills;
pub mod slash_commands;
pub mod source_info;
pub mod system_prompt;
pub mod team;
pub mod timings;
pub mod tools;
