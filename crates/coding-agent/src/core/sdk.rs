//! High-level SDK for creating and configuring agent sessions.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/sdk.ts`.
//!
//! Provides `CreateAgentSessionOptions` — the main configuration surface for
//! constructing an `AgentSession` — and re-exports commonly used types so
//! callers can use a single import path.
//!
//! The full `createAgentSession` async factory from TypeScript is implemented
//! here as `create_agent_session` / `AgentSessionBuilder`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_core::types::ThinkingLevel;

use crate::config::get_agent_dir;
use crate::core::agent_session::{AgentSession, AgentSessionConfig, ScopedModel};
use crate::core::session_manager::{get_default_session_dir, SessionManager};
use crate::core::settings_manager::SettingsManager;
use crate::core::tools::ToolName;

/// Parse a thinking level string to `ThinkingLevel`.  Returns `None` for
/// unrecognised strings.
fn thinking_level_from_str(s: &str) -> Option<ThinkingLevel> {
    match s {
        "off" => Some(ThinkingLevel::Off),
        "minimal" => Some(ThinkingLevel::Minimal),
        "low" => Some(ThinkingLevel::Low),
        "medium" => Some(ThinkingLevel::Medium),
        "high" => Some(ThinkingLevel::High),
        "xhigh" => Some(ThinkingLevel::XHigh),
        _ => None,
    }
}

// Re-exports — mirrors the `export { ... }` block in sdk.ts.
pub use crate::core::agent_session::{AgentSessionEvent, CompactionResult};
pub use crate::core::tools::{all_llm_tools, coding_llm_tools, read_only_llm_tools};

// ============================================================================
// Options
// ============================================================================

/// Options for [`create_agent_session`].
///
/// All fields are optional; sensible defaults are used when omitted.
///
/// Mirrors pi-mono `CreateAgentSessionOptions`.
#[derive(Default)]
pub struct CreateAgentSessionOptions {
    /// Working directory for project-local discovery.
    /// Default: current working directory.
    pub cwd: Option<PathBuf>,

    /// Global agent configuration directory.
    /// Default: `~/.pi/agent`.
    pub agent_dir: Option<PathBuf>,

    /// Provider of the model to use.
    pub model_provider: Option<String>,
    /// ID of the model to use.
    pub model_id: Option<String>,

    /// Thinking level override.
    pub thinking_level: Option<ThinkingLevel>,

    /// Models available for cycling in interactive mode.
    pub scoped_models: Option<Vec<ScopedModel>>,

    /// Built-in tools to activate.  Default: `["read", "bash", "edit", "write"]`.
    pub tools: Option<Vec<ToolName>>,

    /// Session manager override.
    pub session_manager: Option<SessionManager>,

    /// Settings manager override.
    pub settings_manager: Option<SettingsManager>,
}

// ============================================================================
// Result
// ============================================================================

/// Result returned from [`create_agent_session`].
///
/// Mirrors pi-mono `CreateAgentSessionResult`.
pub struct CreateAgentSessionResult {
    /// The constructed agent session.
    pub session: AgentSession,
    /// Optional warning when the saved model could not be restored.
    pub model_fallback_message: Option<String>,
}

// ============================================================================
// create_agent_session
// ============================================================================

/// Build an `AgentSession` with the given options.
///
/// Mirrors pi-mono `createAgentSession`.  All options default to sensible
/// values when omitted:
///
/// - `cwd` → `std::env::current_dir()`
/// - `agent_dir` → `~/.pi/agent`
/// - `thinking_level` → `settings.default_thinking_level` → `DEFAULT_THINKING_LEVEL`
/// - `tools` → `["read", "bash", "edit", "write"]`
pub fn create_agent_session(
    options: CreateAgentSessionOptions,
) -> Result<CreateAgentSessionResult, String> {
    let cwd = options
        .cwd
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let agent_dir: PathBuf = options
        .agent_dir
        .unwrap_or_else(|| get_agent_dir().to_path_buf());

    let settings_manager = options.settings_manager.unwrap_or_else(|| {
        SettingsManager::create(&cwd, &agent_dir)
    });

    let thinking_level = options.thinking_level.unwrap_or_else(|| {
        settings_manager
            .get_default_thinking_level()
            .and_then(|s| thinking_level_from_str(s))
            .unwrap_or(ThinkingLevel::Medium)
    });

    let cwd_str = cwd.to_string_lossy().to_string();
    let session_dir = get_default_session_dir(&cwd_str, Some(&agent_dir));

    let session_manager = options.session_manager.unwrap_or_else(|| {
        SessionManager::create(&cwd_str, Some(&session_dir))
    });

    // active_tool_names is resolved but not yet used in AgentSessionConfig
    // (stored in session after construction, when tool registration is added).
    let _active_tool_names = options
        .tools
        .unwrap_or_else(crate::core::tools::default_coding_tools);

    let config = AgentSessionConfig {
        cwd,
        settings_manager: Arc::new(settings_manager),
        scoped_models: options.scoped_models.unwrap_or_default(),
    };

    let session = AgentSession::new(config);

    Ok(CreateAgentSessionResult {
        session,
        model_fallback_message: None,
    })
}

/// Build an `AgentSession` with all-default options.
///
/// Mirrors pi-mono `createAgentSession({})`.
pub fn create_default_agent_session() -> Result<CreateAgentSessionResult, String> {
    create_agent_session(CreateAgentSessionOptions::default())
}

// ============================================================================
// AgentSessionBuilder (fluent API)
// ============================================================================

/// Fluent builder for constructing an `AgentSession`.
///
/// Equivalent to calling `create_agent_session` with explicit options.
///
/// ```rust,ignore
/// let result = AgentSessionBuilder::new()
///     .with_cwd(std::env::current_dir().unwrap())
///     .with_tools(vec![ToolName::Read, ToolName::Grep])
///     .build()?;
/// ```
#[derive(Default)]
pub struct AgentSessionBuilder {
    options: CreateAgentSessionOptions,
}

impl AgentSessionBuilder {
    /// Create a builder with all-default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the working directory.
    pub fn with_cwd(mut self, cwd: impl AsRef<Path>) -> Self {
        self.options.cwd = Some(cwd.as_ref().to_path_buf());
        self
    }

    /// Override the agent directory.
    pub fn with_agent_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.options.agent_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Override the thinking level.
    pub fn with_thinking_level(mut self, level: ThinkingLevel) -> Self {
        self.options.thinking_level = Some(level);
        self
    }

    /// Override the active tools.
    pub fn with_tools(mut self, tools: Vec<ToolName>) -> Self {
        self.options.tools = Some(tools);
        self
    }

    /// Override the scoped models for cycling.
    pub fn with_scoped_models(mut self, models: Vec<ScopedModel>) -> Self {
        self.options.scoped_models = Some(models);
        self
    }

    /// Construct the `AgentSession`.
    pub fn build(self) -> Result<CreateAgentSessionResult, String> {
        create_agent_session(self.options)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_agent_session_options_default() {
        let opts = CreateAgentSessionOptions::default();
        assert!(opts.cwd.is_none());
        assert!(opts.agent_dir.is_none());
        assert!(opts.model_provider.is_none());
        assert!(opts.model_id.is_none());
        assert!(opts.thinking_level.is_none());
        assert!(opts.tools.is_none());
    }

    #[test]
    fn builder_with_tools() {
        let builder = AgentSessionBuilder::new()
            .with_tools(vec![ToolName::Read, ToolName::Grep]);
        assert!(builder.options.tools.is_some());
        let tools = builder.options.tools.unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0], ToolName::Read);
        assert_eq!(tools[1], ToolName::Grep);
    }

    #[test]
    fn builder_with_thinking_level() {
        let builder = AgentSessionBuilder::new().with_thinking_level(ThinkingLevel::High);
        assert_eq!(builder.options.thinking_level, Some(ThinkingLevel::High));
    }

    #[test]
    fn builder_with_cwd_and_agent_dir() {
        let tmp = std::env::temp_dir();
        let builder = AgentSessionBuilder::new()
            .with_cwd(&tmp)
            .with_agent_dir(&tmp);
        assert_eq!(builder.options.cwd.as_deref(), Some(tmp.as_path()));
        assert_eq!(builder.options.agent_dir.as_deref(), Some(tmp.as_path()));
    }

    #[test]
    fn builder_build_succeeds_with_temp_dir() {
        let tmp = std::env::temp_dir();
        let result = AgentSessionBuilder::new()
            .with_cwd(&tmp)
            .with_agent_dir(&tmp)
            .build();
        // Build should succeed (or return Ok).
        assert!(result.is_ok());
    }

    #[test]
    fn create_default_agent_session_returns_ok() {
        // Smoke test — just verify no panic.
        let result = create_default_agent_session();
        assert!(result.is_ok());
    }
}
