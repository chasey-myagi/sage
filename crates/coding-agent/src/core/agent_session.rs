//! AgentSession — core abstraction for agent lifecycle and session management.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/agent-session.ts`.
//!
//! This struct is shared between all run modes (interactive, print, rpc).
//! It encapsulates:
//! - Agent state access
//! - Event subscription with automatic session persistence
//! - Model and thinking level management
//! - Compaction (manual and auto)
//! - Bash execution
//! - Session switching and branching

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex, broadcast};
use tokio_util::sync::CancellationToken;

use agent_core::event::AgentEvent;
use agent_core::types::{
    AgentMessage, AgentTool, AgentToolResult, AssistantMessage, Content, ThinkingLevel, UserMessage,
};

use crate::core::settings_manager::SettingsManager;

// ============================================================================
// Skill Block Parsing
// ============================================================================

/// Parsed skill block from a user message.
///
/// Mirrors pi-mono's `ParsedSkillBlock` interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedSkillBlock {
    pub name: String,
    pub location: String,
    pub content: String,
    pub user_message: Option<String>,
}

/// Parse a skill block from message text.
/// Returns `None` if the text doesn't contain a skill block.
///
/// Mirrors pi-mono's `parseSkillBlock` function.
pub fn parse_skill_block(text: &str) -> Option<ParsedSkillBlock> {
    // Match: <skill name="..." location="...">\n...\n</skill>(\n\n...)?
    let re = regex::Regex::new(
        r#"(?s)^<skill name="([^"]+)" location="([^"]+)">\n(.*?)\n</skill>(?:\n\n([\s\S]+))?$"#,
    )
    .ok()?;
    let caps = re.captures(text)?;
    Some(ParsedSkillBlock {
        name: caps.get(1)?.as_str().to_string(),
        location: caps.get(2)?.as_str().to_string(),
        content: caps.get(3)?.as_str().to_string(),
        user_message: caps
            .get(4)
            .map(|m| m.as_str().trim().to_string())
            .filter(|s| !s.is_empty()),
    })
}

// ============================================================================
// Event Types
// ============================================================================

/// Compaction result returned from compact / auto-compaction.
///
/// Mirrors pi-mono's `CompactionResult`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResult {
    pub summary: String,
    pub first_kept_entry_id: String,
    pub tokens_before: u64,
    pub details: Value,
}

/// Session-specific events that extend the core AgentEvent.
///
/// Mirrors pi-mono's `AgentSessionEvent` type union.
#[derive(Debug, Clone)]
pub enum AgentSessionEvent {
    /// Pass-through core agent event.
    Core(AgentEvent),
    /// Auto-compaction has started.
    AutoCompactionStart { reason: CompactionReason },
    /// Auto-compaction has ended.
    AutoCompactionEnd {
        result: Option<CompactionResult>,
        aborted: bool,
        will_retry: bool,
        error_message: Option<String>,
    },
    /// Auto-retry is starting.
    AutoRetryStart {
        attempt: u32,
        max_attempts: u32,
        delay_ms: u64,
        error_message: String,
    },
    /// Auto-retry has ended.
    AutoRetryEnd {
        success: bool,
        attempt: u32,
        final_error: Option<String>,
    },
}

/// Reason that triggered auto-compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionReason {
    Threshold,
    Overflow,
}

// ============================================================================
// Config / Options Types
// ============================================================================

/// Configuration for constructing an `AgentSession`.
///
/// Mirrors pi-mono's `AgentSessionConfig` interface.
pub struct AgentSessionConfig {
    /// Working directory.
    pub cwd: PathBuf,
    /// Settings manager.
    pub settings_manager: Arc<SettingsManager>,
    /// Initial scoped models for cycling (from --models flag).
    pub scoped_models: Vec<ScopedModel>,
}

/// A model + optional thinking level pair for cycling.
#[derive(Debug, Clone)]
pub struct ScopedModel {
    pub model_provider: String,
    pub model_id: String,
    pub thinking_level: Option<ThinkingLevel>,
}

/// Options for `AgentSession::prompt()`.
///
/// Mirrors pi-mono's `PromptOptions` interface.
#[derive(Debug, Default, Clone)]
pub struct PromptOptions {
    /// Whether to expand file-based prompt templates (default: true).
    pub expand_prompt_templates: Option<bool>,
    /// When streaming, how to queue the message.
    pub streaming_behavior: Option<StreamingBehavior>,
}

/// How to queue a message when the agent is streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingBehavior {
    Steer,
    FollowUp,
}

/// Direction for model cycling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleDirection {
    Forward,
    Backward,
}

/// Result from `cycle_model()`.
///
/// Mirrors pi-mono's `ModelCycleResult` interface.
#[derive(Debug, Clone)]
pub struct ModelCycleResult {
    pub model_provider: String,
    pub model_id: String,
    pub thinking_level: ThinkingLevel,
    /// Whether cycling through scoped models (--models flag) or all available.
    pub is_scoped: bool,
}

/// Session statistics for /session command.
///
/// Mirrors pi-mono's `SessionStats` interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStats {
    pub session_id: String,
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub tool_calls: usize,
    pub tool_results: usize,
    pub total_messages: usize,
    pub tokens_input: u64,
    pub tokens_output: u64,
    pub tokens_cache_read: u64,
    pub tokens_cache_write: u64,
    pub tokens_total: u64,
    pub cost: f64,
}

/// Context usage information.
///
/// Mirrors pi-mono's `ContextUsage` interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextUsage {
    /// `None` means unknown (e.g. after compaction, waiting for next LLM response).
    pub tokens: Option<u64>,
    pub context_window: u64,
    /// `None` when tokens is `None`.
    pub percent: Option<f64>,
}

// ============================================================================
// Thinking Level Constants
// ============================================================================

/// Standard thinking levels (mirrors pi-mono THINKING_LEVELS).
const THINKING_LEVELS: &[ThinkingLevel] = &[
    ThinkingLevel::Off,
    ThinkingLevel::Minimal,
    ThinkingLevel::Low,
    ThinkingLevel::Medium,
    ThinkingLevel::High,
];

/// Thinking levels including XHigh (for supported models).
const THINKING_LEVELS_WITH_XHIGH: &[ThinkingLevel] = &[
    ThinkingLevel::Off,
    ThinkingLevel::Minimal,
    ThinkingLevel::Low,
    ThinkingLevel::Medium,
    ThinkingLevel::High,
    ThinkingLevel::XHigh,
];

// ============================================================================
// AgentSession
// ============================================================================

/// Agent state snapshot (read-only view).
///
/// Mirrors pi-mono's `AgentState` interface (subset needed by `AgentSession`).
#[derive(Debug, Clone)]
pub struct AgentSessionState {
    pub system_prompt: String,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub thinking_level: ThinkingLevel,
    pub is_streaming: bool,
    pub messages: Vec<AgentMessage>,
}

/// Core abstraction for agent lifecycle and session management.
///
/// Translated from pi-mono `AgentSession` class.
pub struct AgentSession {
    // ── Configuration ──────────────────────────────────────────────────────
    cwd: PathBuf,
    settings_manager: Arc<SettingsManager>,
    scoped_models: Vec<ScopedModel>,

    // ── Internal state ─────────────────────────────────────────────────────
    /// Tracks pending steering messages for UI display.
    steering_messages: Vec<String>,
    /// Tracks pending follow-up messages for UI display.
    follow_up_messages: Vec<String>,

    // ── Compaction state ───────────────────────────────────────────────────
    compaction_token: Option<CancellationToken>,
    auto_compaction_token: Option<CancellationToken>,
    overflow_recovery_attempted: bool,

    // ── Branch summarization state ─────────────────────────────────────────
    branch_summary_token: Option<CancellationToken>,

    // ── Retry state ────────────────────────────────────────────────────────
    retry_token: Option<CancellationToken>,
    retry_attempt: u32,

    // ── Bash execution state ───────────────────────────────────────────────
    bash_token: Option<CancellationToken>,

    // ── Event broadcasting ─────────────────────────────────────────────────
    event_tx: broadcast::Sender<AgentSessionEvent>,

    // ── Tool registry ──────────────────────────────────────────────────────
    /// name → (description, prompt_snippet)
    tool_registry: HashMap<String, ToolRegistryEntry>,

    // ── Session data (in-memory stub) ──────────────────────────────────────
    session_id: String,
    messages: Vec<AgentMessage>,
    system_prompt: String,
    model_provider: Option<String>,
    model_id: Option<String>,
    thinking_level: ThinkingLevel,
    is_streaming: bool,
    turn_index: u32,
    base_system_prompt: String,
    last_assistant_message: Option<AssistantMessage>,
}

/// Entry in the tool registry.
#[derive(Debug, Clone)]
struct ToolRegistryEntry {
    pub description: String,
    pub prompt_snippet: Option<String>,
    pub prompt_guidelines: Vec<String>,
}

impl AgentSession {
    // =========================================================================
    // Construction
    // =========================================================================

    /// Create a new `AgentSession`.
    ///
    /// Mirrors pi-mono's `AgentSession` constructor.
    pub fn new(config: AgentSessionConfig) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            cwd: config.cwd,
            settings_manager: config.settings_manager,
            scoped_models: config.scoped_models,
            steering_messages: Vec::new(),
            follow_up_messages: Vec::new(),
            compaction_token: None,
            auto_compaction_token: None,
            overflow_recovery_attempted: false,
            branch_summary_token: None,
            retry_token: None,
            retry_attempt: 0,
            bash_token: None,
            event_tx,
            tool_registry: HashMap::new(),
            session_id: ulid::Ulid::new().to_string(),
            messages: Vec::new(),
            system_prompt: String::new(),
            model_provider: None,
            model_id: None,
            thinking_level: ThinkingLevel::Off,
            is_streaming: false,
            turn_index: 0,
            base_system_prompt: String::new(),
            last_assistant_message: None,
        }
    }

    // =========================================================================
    // Event Subscription
    // =========================================================================

    /// Subscribe to agent session events.
    /// Returns a `broadcast::Receiver`. Drop it to unsubscribe.
    ///
    /// Mirrors pi-mono's `subscribe(listener)`.
    pub fn subscribe(&self) -> broadcast::Receiver<AgentSessionEvent> {
        self.event_tx.subscribe()
    }

    /// Emit an event to all subscribers.
    ///
    /// Mirrors pi-mono's `_emit(event)`.
    fn emit(&self, event: AgentSessionEvent) {
        // Ignore send errors (no subscribers is fine).
        let _ = self.event_tx.send(event);
    }

    /// Process an agent event received from the underlying agent loop.
    ///
    /// Mirrors pi-mono's `_processAgentEvent(event)`.
    pub async fn handle_agent_event(&mut self, event: AgentEvent) {
        // When a user message starts, remove it from steering/follow-up queues.
        if let AgentEvent::MessageStart { ref message } = event {
            if let AgentMessage::User(user_msg) = message {
                let text = user_msg
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<String>();
                if !text.is_empty() {
                    // Check steering queue first, then follow-up queue.
                    if let Some(pos) = self.steering_messages.iter().position(|m| m == &text) {
                        self.steering_messages.remove(pos);
                    } else if let Some(pos) =
                        self.follow_up_messages.iter().position(|m| m == &text)
                    {
                        self.follow_up_messages.remove(pos);
                    }
                }
            }
        }

        // Forward to all subscribers.
        self.emit(AgentSessionEvent::Core(event.clone()));

        // Track assistant messages for auto-compaction.
        if let AgentEvent::MessageEnd { ref message } = event {
            if let AgentMessage::Assistant(assistant) = message {
                self.last_assistant_message = Some(assistant.clone());

                // Reset overflow flag on success.
                if assistant.stop_reason != agent_core::types::StopReason::Error {
                    self.overflow_recovery_attempted = false;
                }

                // Reset retry counter on successful response.
                if assistant.stop_reason != agent_core::types::StopReason::Error
                    && self.retry_attempt > 0
                {
                    self.emit(AgentSessionEvent::AutoRetryEnd {
                        success: true,
                        attempt: self.retry_attempt,
                        final_error: None,
                    });
                    self.retry_attempt = 0;
                }
            }
        }

        // Check auto-retry and auto-compaction after agent completes.
        if let AgentEvent::AgentEnd { .. } = event {
            if let Some(msg) = self.last_assistant_message.take() {
                if self.is_retryable_error(&msg) {
                    let did_retry = self.handle_retryable_error(&msg).await;
                    if did_retry {
                        return;
                    }
                }
                self.check_compaction(&msg, true).await;
            }
        }
    }

    // =========================================================================
    // State Access
    // =========================================================================

    /// Current model provider, or `None` if not yet selected.
    pub fn model_provider(&self) -> Option<&str> {
        self.model_provider.as_deref()
    }

    /// Current model ID, or `None` if not yet selected.
    pub fn model_id(&self) -> Option<&str> {
        self.model_id.as_deref()
    }

    /// Current thinking level.
    pub fn thinking_level(&self) -> ThinkingLevel {
        self.thinking_level
    }

    /// Whether the agent is currently streaming a response.
    pub fn is_streaming(&self) -> bool {
        self.is_streaming
    }

    /// Current effective system prompt.
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    /// Current retry attempt (0 if not retrying).
    pub fn retry_attempt(&self) -> u32 {
        self.retry_attempt
    }

    /// All messages in the current session.
    pub fn messages(&self) -> &[AgentMessage] {
        &self.messages
    }

    /// Current session ID.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Scoped models for cycling.
    pub fn scoped_models(&self) -> &[ScopedModel] {
        &self.scoped_models
    }

    /// Update scoped models.
    pub fn set_scoped_models(&mut self, models: Vec<ScopedModel>) {
        self.scoped_models = models;
    }

    /// Whether compaction or branch summarization is currently running.
    pub fn is_compacting(&self) -> bool {
        self.auto_compaction_token.is_some()
            || self.compaction_token.is_some()
            || self.branch_summary_token.is_some()
    }

    /// Whether auto-retry is currently in progress.
    pub fn is_retrying(&self) -> bool {
        self.retry_token.is_some()
    }

    /// Whether a bash command is currently running.
    pub fn is_bash_running(&self) -> bool {
        self.bash_token.is_some()
    }

    /// Number of pending messages (steering + follow-up).
    pub fn pending_message_count(&self) -> usize {
        self.steering_messages.len() + self.follow_up_messages.len()
    }

    /// Read-only view of pending steering messages.
    pub fn steering_messages(&self) -> &[String] {
        &self.steering_messages
    }

    /// Read-only view of pending follow-up messages.
    pub fn follow_up_messages(&self) -> &[String] {
        &self.follow_up_messages
    }

    /// Whether auto-compaction is enabled.
    pub fn auto_compaction_enabled(&self) -> bool {
        self.settings_manager.get_compaction_enabled()
    }

    /// Enable or disable auto-compaction.
    ///
    /// Mirrors pi-mono's `setAutoCompactionEnabled(enabled)`.
    pub fn set_auto_compaction_enabled(&mut self, _enabled: bool) {
        // TODO: settings_manager is Arc<SettingsManager>; migrate to Arc<Mutex<>> for mutable access.
    }

    /// Whether auto-retry is enabled.
    pub fn auto_retry_enabled(&self) -> bool {
        self.settings_manager.get_retry_enabled()
    }

    /// Enable or disable auto-retry.
    ///
    /// Mirrors pi-mono's `setAutoRetryEnabled(enabled)`.
    pub fn set_auto_retry_enabled(&mut self, _enabled: bool) {
        // TODO: settings_manager is Arc<SettingsManager>; migrate to Arc<Mutex<>> for mutable access.
    }

    // =========================================================================
    // Tool Registry
    // =========================================================================

    /// Register a tool in the session tool registry.
    pub fn register_tool(
        &mut self,
        name: String,
        description: String,
        prompt_snippet: Option<String>,
        prompt_guidelines: Vec<String>,
    ) {
        self.tool_registry.insert(
            name,
            ToolRegistryEntry {
                description,
                prompt_snippet,
                prompt_guidelines,
            },
        );
    }

    /// Get the names of currently active tools.
    ///
    /// Mirrors pi-mono's `getActiveToolNames()`.
    pub fn get_active_tool_names(&self) -> Vec<String> {
        self.tool_registry.keys().cloned().collect()
    }

    // =========================================================================
    // Thinking Level Management
    // =========================================================================

    /// Set thinking level, clamping to model capabilities.
    ///
    /// Mirrors pi-mono's `setThinkingLevel(level)`.
    pub fn set_thinking_level(&mut self, level: ThinkingLevel) {
        let available = self.get_available_thinking_levels();
        let effective = if available.contains(&level) {
            level
        } else {
            self.clamp_thinking_level(level, &available)
        };

        let is_changing = effective != self.thinking_level;
        self.thinking_level = effective;

        if is_changing {
            // Persist to settings.
            // TODO: settings_manager is Arc<SettingsManager>; migrate to Arc<Mutex<>> for mutable access.
            let _ = effective;
        }
    }

    /// Cycle to the next thinking level.
    /// Returns `None` if the model doesn't support thinking.
    ///
    /// Mirrors pi-mono's `cycleThinkingLevel()`.
    pub fn cycle_thinking_level(&mut self) -> Option<ThinkingLevel> {
        if !self.supports_thinking() {
            return None;
        }

        let levels = self.get_available_thinking_levels();
        let current_index = levels
            .iter()
            .position(|&l| l == self.thinking_level)
            .unwrap_or(0);
        let next_index = (current_index + 1) % levels.len();
        let next = levels[next_index];
        self.set_thinking_level(next);
        Some(next)
    }

    /// Get available thinking levels for the current model.
    ///
    /// Mirrors pi-mono's `getAvailableThinkingLevels()`.
    pub fn get_available_thinking_levels(&self) -> Vec<ThinkingLevel> {
        if !self.supports_thinking() {
            return vec![ThinkingLevel::Off];
        }
        if self.supports_xhigh_thinking() {
            THINKING_LEVELS_WITH_XHIGH.to_vec()
        } else {
            THINKING_LEVELS.to_vec()
        }
    }

    /// Whether the current model supports the xhigh thinking level.
    ///
    /// Mirrors pi-mono's `supportsXhighThinking()`.
    pub fn supports_xhigh_thinking(&self) -> bool {
        // Stub: delegate to model capability check when model registry is available.
        false
    }

    /// Whether the current model supports thinking/reasoning.
    ///
    /// Mirrors pi-mono's `supportsThinking()`.
    pub fn supports_thinking(&self) -> bool {
        // Stub: delegate to model capability check.
        false
    }

    /// Clamp a thinking level to the closest available level.
    ///
    /// Mirrors pi-mono's `_clampThinkingLevel`.
    fn clamp_thinking_level(
        &self,
        level: ThinkingLevel,
        available: &[ThinkingLevel],
    ) -> ThinkingLevel {
        let ordered = THINKING_LEVELS_WITH_XHIGH;
        let requested_index = ordered.iter().position(|&l| l == level);

        if let Some(idx) = requested_index {
            // Try to find nearest available at or above the requested level.
            for i in idx..ordered.len() {
                if available.contains(&ordered[i]) {
                    return ordered[i];
                }
            }
            // Fall back to nearest below.
            for i in (0..idx).rev() {
                if available.contains(&ordered[i]) {
                    return ordered[i];
                }
            }
        }

        available.first().copied().unwrap_or(ThinkingLevel::Off)
    }

    /// Get the thinking level to use when switching models.
    ///
    /// Mirrors pi-mono's `_getThinkingLevelForModelSwitch`.
    fn get_thinking_level_for_model_switch(
        &self,
        explicit: Option<ThinkingLevel>,
    ) -> ThinkingLevel {
        if let Some(level) = explicit {
            return level;
        }
        if !self.supports_thinking() {
            // get_default_thinking_level returns Option<&str> — parse via serde_json.
            return self
                .settings_manager
                .get_default_thinking_level()
                .and_then(|s| {
                    serde_json::from_value::<ThinkingLevel>(serde_json::Value::String(s.to_owned()))
                        .ok()
                })
                .unwrap_or(ThinkingLevel::Off);
        }
        self.thinking_level
    }

    // =========================================================================
    // Queue Mode Management
    // =========================================================================

    /// Set steering message mode and persist to settings.
    ///
    /// Mirrors pi-mono's `setSteeringMode(mode)`.
    pub fn set_steering_mode(&mut self, _mode: &str) {
        // TODO: settings_manager is Arc<SettingsManager>; migrate to Arc<Mutex<>> for mutable access.
    }

    /// Set follow-up message mode and persist to settings.
    ///
    /// Mirrors pi-mono's `setFollowUpMode(mode)`.
    pub fn set_follow_up_mode(&mut self, _mode: &str) {
        // TODO: settings_manager is Arc<SettingsManager>; migrate to Arc<Mutex<>> for mutable access.
    }

    // =========================================================================
    // Prompting
    // =========================================================================

    /// Send a prompt to the agent.
    ///
    /// Mirrors pi-mono's `prompt(text, options)`.
    pub async fn prompt(
        &mut self,
        text: &str,
        options: Option<PromptOptions>,
    ) -> Result<(), String> {
        let _options = options.unwrap_or_default();

        if self.is_streaming {
            return Err(
                "Agent is already processing. Specify streamingBehavior ('steer' or 'followUp') to queue the message."
                    .to_string(),
            );
        }

        if self.model_provider.is_none() {
            return Err("No model selected.".to_string());
        }

        // Build user message.
        let user_msg = UserMessage {
            content: vec![Content::Text {
                text: text.to_string(),
            }],
            timestamp: {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
            },
        };
        self.messages.push(AgentMessage::User(user_msg));

        Ok(())
    }

    /// Queue a steering message while the agent is running.
    ///
    /// Mirrors pi-mono's `steer(text, images?)`.
    pub fn steer(&mut self, text: String) {
        self.steering_messages.push(text.clone());
    }

    /// Queue a follow-up message to be processed after the agent finishes.
    ///
    /// Mirrors pi-mono's `followUp(text, images?)`.
    pub fn follow_up(&mut self, text: String) {
        self.follow_up_messages.push(text.clone());
    }

    /// Clear all queued messages and return them.
    ///
    /// Mirrors pi-mono's `clearQueue()`.
    pub fn clear_queue(&mut self) -> (Vec<String>, Vec<String>) {
        let steering = std::mem::take(&mut self.steering_messages);
        let follow_up = std::mem::take(&mut self.follow_up_messages);
        (steering, follow_up)
    }

    // =========================================================================
    // Session Management
    // =========================================================================

    /// Start a new session, clearing all messages.
    ///
    /// Mirrors pi-mono's `newSession(options?)`.
    pub async fn new_session(&mut self) -> bool {
        self.messages.clear();
        self.steering_messages.clear();
        self.follow_up_messages.clear();
        self.session_id = ulid::Ulid::new().to_string();
        self.overflow_recovery_attempted = false;
        true
    }

    /// Set a display name for the current session.
    ///
    /// Mirrors pi-mono's `setSessionName(name)`.
    pub fn set_session_name(&mut self, _name: &str) {
        // Stub: delegate to session manager when available.
        todo!("session manager not yet wired in")
    }

    /// Switch to a different session file.
    ///
    /// Mirrors pi-mono's `switchSession(sessionPath)`.
    pub async fn switch_session(&mut self, _session_path: &str) -> bool {
        // Stub: delegate to session manager when available.
        todo!("session manager not yet wired in")
    }

    /// Create a fork from a specific entry.
    ///
    /// Mirrors pi-mono's `fork(entryId)`.
    pub async fn fork(&mut self, _entry_id: &str) -> Result<ForkResult, String> {
        // Stub: delegate to session manager when available.
        todo!("session manager not yet wired in")
    }

    /// Navigate to a different node in the session tree.
    ///
    /// Mirrors pi-mono's `navigateTree(targetId, options)`.
    pub async fn navigate_tree(
        &mut self,
        _target_id: &str,
        _options: NavigateTreeOptions,
    ) -> Result<NavigateTreeResult, String> {
        // Stub: delegate to session manager when available.
        todo!("session manager not yet wired in")
    }

    /// Get all user messages from session for fork selector.
    ///
    /// Mirrors pi-mono's `getUserMessagesForForking()`.
    pub fn get_user_messages_for_forking(&self) -> Vec<UserMessageEntry> {
        let mut result = Vec::new();
        for (i, msg) in self.messages.iter().enumerate() {
            if let AgentMessage::User(user_msg) = msg {
                let text = user_msg
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<String>();
                if !text.is_empty() {
                    result.push(UserMessageEntry {
                        entry_id: i.to_string(),
                        text,
                    });
                }
            }
        }
        result
    }

    // =========================================================================
    // Model Management
    // =========================================================================

    /// Set model directly.
    ///
    /// Mirrors pi-mono's `setModel(model)`.
    pub async fn set_model(&mut self, provider: String, model_id: String) -> Result<(), String> {
        self.model_provider = Some(provider.clone());
        self.model_id = Some(model_id.clone());

        // Re-clamp thinking level for new model capabilities.
        let level = self.get_thinking_level_for_model_switch(None);
        self.set_thinking_level(level);

        Ok(())
    }

    /// Cycle to next/previous model.
    ///
    /// Mirrors pi-mono's `cycleModel(direction)`.
    pub async fn cycle_model(&mut self, direction: CycleDirection) -> Option<ModelCycleResult> {
        if !self.scoped_models.is_empty() {
            return self.cycle_scoped_model(direction).await;
        }
        None
    }

    async fn cycle_scoped_model(&mut self, direction: CycleDirection) -> Option<ModelCycleResult> {
        if self.scoped_models.len() <= 1 {
            return None;
        }
        let current_index = self
            .scoped_models
            .iter()
            .position(|m| {
                Some(&m.model_provider) == self.model_provider.as_ref()
                    && Some(&m.model_id) == self.model_id.as_ref()
            })
            .unwrap_or(0);

        let len = self.scoped_models.len();
        let next_index = match direction {
            CycleDirection::Forward => (current_index + 1) % len,
            CycleDirection::Backward => (current_index + len - 1) % len,
        };

        let next = self.scoped_models[next_index].clone();
        let thinking_level = self.get_thinking_level_for_model_switch(next.thinking_level);
        self.model_provider = Some(next.model_provider.clone());
        self.model_id = Some(next.model_id.clone());
        self.set_thinking_level(thinking_level);

        Some(ModelCycleResult {
            model_provider: next.model_provider,
            model_id: next.model_id,
            thinking_level: self.thinking_level,
            is_scoped: true,
        })
    }

    // =========================================================================
    // Compaction
    // =========================================================================

    /// Manually compact the session context.
    ///
    /// Mirrors pi-mono's `compact(customInstructions?)`.
    pub async fn compact(
        &mut self,
        _custom_instructions: Option<&str>,
    ) -> Result<CompactionResult, String> {
        if self.model_provider.is_none() {
            return Err("No model selected".to_string());
        }

        self.compaction_token = Some(CancellationToken::new());

        // Stub implementation: real compaction requires session manager + LLM call.
        let result = CompactionResult {
            summary: "Compaction stub".to_string(),
            first_kept_entry_id: "stub".to_string(),
            tokens_before: 0,
            details: Value::Null,
        };

        self.compaction_token = None;
        Ok(result)
    }

    /// Cancel in-progress compaction (manual or auto).
    ///
    /// Mirrors pi-mono's `abortCompaction()`.
    pub fn abort_compaction(&mut self) {
        if let Some(token) = self.compaction_token.take() {
            token.cancel();
        }
        if let Some(token) = self.auto_compaction_token.take() {
            token.cancel();
        }
    }

    /// Cancel in-progress branch summarization.
    ///
    /// Mirrors pi-mono's `abortBranchSummary()`.
    pub fn abort_branch_summary(&mut self) {
        if let Some(token) = self.branch_summary_token.take() {
            token.cancel();
        }
    }

    /// Check if compaction is needed and run it.
    ///
    /// Mirrors pi-mono's `_checkCompaction(assistantMessage, skipAbortedCheck)`.
    async fn check_compaction(&mut self, assistant: &AssistantMessage, skip_aborted_check: bool) {
        let settings = self.settings_manager.as_ref().get_compaction_settings();
        if !settings.enabled.unwrap_or(true) {
            return;
        }

        // Skip aborted messages unless explicitly allowed.
        use agent_core::types::StopReason;
        if skip_aborted_check && assistant.stop_reason == StopReason::Aborted {
            return;
        }

        // Case 1: Overflow — LLM returned context overflow error.
        if self.is_context_overflow(assistant) {
            if self.overflow_recovery_attempted {
                self.emit(AgentSessionEvent::AutoCompactionEnd {
                    result: None,
                    aborted: false,
                    will_retry: false,
                    error_message: Some(
                        "Context overflow recovery failed after one compact-and-retry attempt. \
                         Try reducing context or switching to a larger-context model."
                            .to_string(),
                    ),
                });
                return;
            }
            self.overflow_recovery_attempted = true;
            // Remove error message from agent state.
            if let Some(AgentMessage::Assistant(_)) = self.messages.last() {
                self.messages.pop();
            }
            self.run_auto_compaction(CompactionReason::Overflow, true)
                .await;
            return;
        }

        // Case 2: Threshold — context is getting large.
        // Stub: real implementation needs token counting.
    }

    /// Run auto-compaction with events.
    ///
    /// Mirrors pi-mono's `_runAutoCompaction(reason, willRetry)`.
    async fn run_auto_compaction(&mut self, reason: CompactionReason, will_retry: bool) {
        self.emit(AgentSessionEvent::AutoCompactionStart { reason });
        self.auto_compaction_token = Some(CancellationToken::new());

        if self.model_provider.is_none() {
            self.emit(AgentSessionEvent::AutoCompactionEnd {
                result: None,
                aborted: false,
                will_retry: false,
                error_message: None,
            });
            self.auto_compaction_token = None;
            return;
        }

        // Stub: real compaction requires session manager + LLM call.
        let result = CompactionResult {
            summary: "Auto-compaction stub".to_string(),
            first_kept_entry_id: "stub".to_string(),
            tokens_before: 0,
            details: Value::Null,
        };

        self.emit(AgentSessionEvent::AutoCompactionEnd {
            result: Some(result),
            aborted: false,
            will_retry,
            error_message: None,
        });

        self.auto_compaction_token = None;
    }

    /// Check whether an assistant message represents a context overflow error.
    fn is_context_overflow(&self, msg: &AssistantMessage) -> bool {
        use agent_core::types::StopReason;
        if msg.stop_reason != StopReason::Error {
            return false;
        }
        if let Some(ref err) = msg.error_message {
            // Common overflow error patterns.
            let lower = err.to_lowercase();
            lower.contains("prompt is too long")
                || lower.contains("context window")
                || lower.contains("too many tokens")
                || lower.contains("maximum context")
        } else {
            false
        }
    }

    // =========================================================================
    // Auto-Retry
    // =========================================================================

    /// Check if an error is retryable.
    ///
    /// Mirrors pi-mono's `_isRetryableError(message)`.
    fn is_retryable_error(&self, msg: &AssistantMessage) -> bool {
        use agent_core::types::StopReason;
        if msg.stop_reason != StopReason::Error {
            return false;
        }
        if self.is_context_overflow(msg) {
            return false;
        }
        if let Some(ref err) = msg.error_message {
            let re = regex::Regex::new(
                r"(?i)overloaded|provider.?returned.?error|rate.?limit|too many requests|429|500|502|503|504|service.?unavailable|server.?error|internal.?error|network.?error|connection.?error|connection.?refused|other side closed|fetch failed|upstream.?connect|reset before headers|socket hang up|timed? out|timeout|terminated|retry delay",
            )
            .expect("static retry regex is valid");
            re.is_match(err)
        } else {
            false
        }
    }

    /// Handle retryable errors with exponential backoff.
    /// Returns `true` if retry was initiated.
    ///
    /// Mirrors pi-mono's `_handleRetryableError(message)`.
    async fn handle_retryable_error(&mut self, msg: &AssistantMessage) -> bool {
        let settings = self.settings_manager.as_ref().get_retry_settings();
        if !settings.enabled.unwrap_or(true) {
            return false;
        }

        self.retry_attempt += 1;
        let max_retries = settings.max_retries.unwrap_or(3);

        if self.retry_attempt > max_retries {
            self.emit(AgentSessionEvent::AutoRetryEnd {
                success: false,
                attempt: self.retry_attempt - 1,
                final_error: msg.error_message.clone(),
            });
            self.retry_attempt = 0;
            return false;
        }

        let base_delay_ms = settings.base_delay_ms.unwrap_or(2000);
        let delay_ms = base_delay_ms * 2u64.pow(self.retry_attempt - 1);

        self.emit(AgentSessionEvent::AutoRetryStart {
            attempt: self.retry_attempt,
            max_attempts: max_retries,
            delay_ms,
            error_message: msg
                .error_message
                .clone()
                .unwrap_or_else(|| "Unknown error".to_string()),
        });

        // Remove error message from agent state.
        if let Some(AgentMessage::Assistant(_)) = self.messages.last() {
            self.messages.pop();
        }

        // Set up cancellable sleep.
        let token = CancellationToken::new();
        self.retry_token = Some(token.clone());

        let sleep_result = tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(delay_ms)) => true,
            _ = token.cancelled() => false,
        };

        self.retry_token = None;

        if !sleep_result {
            // Aborted during sleep.
            let attempt = self.retry_attempt;
            self.retry_attempt = 0;
            self.emit(AgentSessionEvent::AutoRetryEnd {
                success: false,
                attempt,
                final_error: Some("Retry cancelled".to_string()),
            });
            return false;
        }

        true
    }

    /// Cancel in-progress retry.
    ///
    /// Mirrors pi-mono's `abortRetry()`.
    pub fn abort_retry(&mut self) {
        if let Some(token) = self.retry_token.take() {
            token.cancel();
        }
    }

    // =========================================================================
    // Bash Execution
    // =========================================================================

    /// Cancel running bash command.
    ///
    /// Mirrors pi-mono's `abortBash()`.
    pub fn abort_bash(&mut self) {
        if let Some(token) = self.bash_token.take() {
            token.cancel();
        }
    }

    // =========================================================================
    // Session Statistics
    // =========================================================================

    /// Get session statistics.
    ///
    /// Mirrors pi-mono's `getSessionStats()`.
    pub fn get_session_stats(&self) -> SessionStats {
        let mut user_messages = 0usize;
        let mut assistant_messages = 0usize;
        let mut tool_calls = 0usize;
        let mut tool_results = 0usize;
        let mut tokens_input = 0u64;
        let mut tokens_output = 0u64;
        let mut tokens_cache_read = 0u64;
        let mut tokens_cache_write = 0u64;
        let mut cost = 0f64;

        for msg in &self.messages {
            match msg {
                AgentMessage::User(_) => user_messages += 1,
                AgentMessage::Assistant(a) => {
                    assistant_messages += 1;
                    tool_calls += a
                        .content
                        .iter()
                        .filter(|c| matches!(c, Content::ToolCall { .. }))
                        .count();
                    tokens_input += a.usage.input;
                    tokens_output += a.usage.output;
                    tokens_cache_read += a.usage.cache_read;
                    tokens_cache_write += a.usage.cache_write;
                    cost += a.usage.cost.total;
                }
                AgentMessage::ToolResult(_) => tool_results += 1,
                AgentMessage::CompactionSummary(_) => {}
            }
        }

        SessionStats {
            session_id: self.session_id.clone(),
            user_messages,
            assistant_messages,
            tool_calls,
            tool_results,
            total_messages: self.messages.len(),
            tokens_input,
            tokens_output,
            tokens_cache_read,
            tokens_cache_write,
            tokens_total: tokens_input + tokens_output + tokens_cache_read + tokens_cache_write,
            cost,
        }
    }

    /// Get context usage.
    ///
    /// Mirrors pi-mono's `getContextUsage()`.
    pub fn get_context_usage(&self) -> Option<ContextUsage> {
        // Stub: requires model context window info.
        None
    }

    // =========================================================================
    // Text Utilities
    // =========================================================================

    /// Extract text content of last assistant message.
    ///
    /// Mirrors pi-mono's `getLastAssistantText()`.
    pub fn get_last_assistant_text(&self) -> Option<String> {
        let last = self.messages.iter().rev().find_map(|m| {
            if let AgentMessage::Assistant(a) = m {
                // Skip aborted messages with no content.
                if a.stop_reason == agent_core::types::StopReason::Aborted && a.content.is_empty() {
                    return None;
                }
                Some(a)
            } else {
                None
            }
        })?;

        let text: String = last
            .content
            .iter()
            .filter_map(|c| match c {
                Content::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();

        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    /// Dispose the session, removing all listeners.
    ///
    /// Mirrors pi-mono's `dispose()`.
    pub fn dispose(&mut self) {
        self.abort_compaction();
        self.abort_branch_summary();
        self.abort_retry();
        self.abort_bash();
        // broadcast::Sender will be dropped when self drops.
    }
}

// ============================================================================
// Helper Types
// ============================================================================

/// Result from `fork()`.
pub struct ForkResult {
    pub selected_text: String,
    pub cancelled: bool,
}

/// Options for `navigate_tree()`.
#[derive(Debug, Default)]
pub struct NavigateTreeOptions {
    pub summarize: Option<bool>,
    pub custom_instructions: Option<String>,
    pub replace_instructions: Option<bool>,
    pub label: Option<String>,
}

/// Result from `navigate_tree()`.
pub struct NavigateTreeResult {
    pub editor_text: Option<String>,
    pub cancelled: bool,
    pub aborted: Option<bool>,
}

/// Entry returned from `get_user_messages_for_forking()`.
#[derive(Debug, Clone)]
pub struct UserMessageEntry {
    pub entry_id: String,
    pub text: String,
}

// agent_session.rs stub methods removed — real implementations now live in settings_manager.rs.

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Create a minimal AgentSession for testing.
    fn make_session() -> AgentSession {
        // Use a temporary directory for settings.
        let tmp = std::env::temp_dir().join(format!("sage-test-{}", std::process::id()));
        let settings_manager = Arc::new(SettingsManager::create(&tmp, &tmp));
        AgentSession::new(AgentSessionConfig {
            cwd: tmp,
            settings_manager,
            scoped_models: vec![],
        })
    }

    // ── parse_skill_block ────────────────────────────────────────────────────

    #[test]
    fn test_parse_skill_block_valid() {
        let text =
            "<skill name=\"my-skill\" location=\"/path/to/skill.md\">\ncontent here\n</skill>";
        let result = parse_skill_block(text);
        assert!(result.is_some());
        let block = result.unwrap();
        assert_eq!(block.name, "my-skill");
        assert_eq!(block.location, "/path/to/skill.md");
        assert_eq!(block.content, "content here");
        assert_eq!(block.user_message, None);
    }

    #[test]
    fn test_parse_skill_block_with_user_message() {
        let text = "<skill name=\"my-skill\" location=\"/path/to/skill.md\">\ncontent here\n</skill>\n\nUser message here";
        let result = parse_skill_block(text);
        assert!(result.is_some());
        let block = result.unwrap();
        assert_eq!(block.user_message, Some("User message here".to_string()));
    }

    #[test]
    fn test_parse_skill_block_invalid() {
        assert!(parse_skill_block("no skill block here").is_none());
        assert!(parse_skill_block("").is_none());
        assert!(parse_skill_block("<skill>unclosed").is_none());
    }

    // ── AgentSession construction ────────────────────────────────────────────

    #[test]
    fn test_new_session_initial_state() {
        let session = make_session();
        assert!(session.model_provider().is_none());
        assert!(session.model_id().is_none());
        assert!(!session.is_streaming());
        assert!(!session.is_compacting());
        assert!(!session.is_retrying());
        assert!(!session.is_bash_running());
        assert_eq!(session.pending_message_count(), 0);
        assert!(session.messages().is_empty());
        assert_eq!(session.retry_attempt(), 0);
    }

    // ── subscribe / emit ─────────────────────────────────────────────────────

    #[test]
    fn test_subscribe_receives_events() {
        let session = make_session();
        let mut rx = session.subscribe();

        // Emit an event directly.
        session.emit(AgentSessionEvent::AutoCompactionStart {
            reason: CompactionReason::Threshold,
        });

        let event = rx.try_recv().expect("should have received event");
        assert!(matches!(
            event,
            AgentSessionEvent::AutoCompactionStart {
                reason: CompactionReason::Threshold
            }
        ));
    }

    // ── steering / follow-up queues ──────────────────────────────────────────

    #[test]
    fn test_steer_adds_to_queue() {
        let mut session = make_session();
        session.steer("do this first".to_string());
        session.steer("do this second".to_string());
        assert_eq!(session.pending_message_count(), 2);
        assert_eq!(
            session.steering_messages(),
            &["do this first", "do this second"]
        );
        assert_eq!(session.follow_up_messages().len(), 0);
    }

    #[test]
    fn test_follow_up_adds_to_queue() {
        let mut session = make_session();
        session.follow_up("follow A".to_string());
        assert_eq!(session.pending_message_count(), 1);
        assert_eq!(session.follow_up_messages(), &["follow A"]);
    }

    #[test]
    fn test_clear_queue_returns_all() {
        let mut session = make_session();
        session.steer("s1".to_string());
        session.follow_up("f1".to_string());
        let (steering, follow_up) = session.clear_queue();
        assert_eq!(steering, vec!["s1"]);
        assert_eq!(follow_up, vec!["f1"]);
        assert_eq!(session.pending_message_count(), 0);
    }

    // ── prompt guard ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_prompt_requires_model() {
        let mut session = make_session();
        // No model set → should error.
        let result = session.prompt("hello", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No model selected"));
    }

    #[tokio::test]
    async fn test_prompt_rejects_while_streaming() {
        let mut session = make_session();
        session.model_provider = Some("anthropic".to_string());
        session.model_id = Some("claude-opus-4-5".to_string());
        session.is_streaming = true;

        let result = session.prompt("hello", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Agent is already processing"));
    }

    // ── thinking level management ────────────────────────────────────────────

    #[test]
    fn test_get_available_thinking_levels_no_thinking() {
        let session = make_session();
        // Default: supports_thinking() == false.
        let levels = session.get_available_thinking_levels();
        assert_eq!(levels, vec![ThinkingLevel::Off]);
    }

    #[test]
    fn test_clamp_thinking_level_exact_match() {
        let session = make_session();
        let available = vec![ThinkingLevel::Off, ThinkingLevel::Low, ThinkingLevel::High];
        let clamped = session.clamp_thinking_level(ThinkingLevel::Low, &available);
        assert_eq!(clamped, ThinkingLevel::Low);
    }

    #[test]
    fn test_clamp_thinking_level_upward_search() {
        // If "Minimal" not available, should find next higher available.
        let session = make_session();
        let available = vec![ThinkingLevel::Off, ThinkingLevel::Low, ThinkingLevel::High];
        let clamped = session.clamp_thinking_level(ThinkingLevel::Minimal, &available);
        assert_eq!(clamped, ThinkingLevel::Low);
    }

    // ── compaction abort ─────────────────────────────────────────────────────

    #[test]
    fn test_abort_compaction_clears_tokens() {
        let mut session = make_session();
        session.compaction_token = Some(CancellationToken::new());
        session.auto_compaction_token = Some(CancellationToken::new());
        session.abort_compaction();
        assert!(session.compaction_token.is_none());
        assert!(session.auto_compaction_token.is_none());
    }

    // ── session stats ────────────────────────────────────────────────────────

    #[test]
    fn test_session_stats_empty() {
        let session = make_session();
        let stats = session.get_session_stats();
        assert_eq!(stats.user_messages, 0);
        assert_eq!(stats.assistant_messages, 0);
        assert_eq!(stats.tool_calls, 0);
        assert_eq!(stats.total_messages, 0);
        assert_eq!(stats.tokens_input, 0);
        assert_eq!(stats.cost, 0.0);
    }

    #[test]
    fn test_session_stats_counts_messages() {
        let mut session = make_session();
        session.messages.push(AgentMessage::User(UserMessage {
            content: vec![Content::Text {
                text: "hi".to_string(),
            }],
            timestamp: 0,
        }));
        session
            .messages
            .push(AgentMessage::Assistant(AssistantMessage {
                content: vec![
                    Content::Text {
                        text: "hello".to_string(),
                    },
                    Content::ToolCall {
                        id: "tc1".to_string(),
                        name: "read".to_string(),
                        arguments: Value::Null,
                    },
                ],
                provider: "anthropic".to_string(),
                model: "claude-opus-4-5".to_string(),
                usage: agent_core::types::Usage::default(),
                stop_reason: agent_core::types::StopReason::ToolUse,
                error_message: None,
                timestamp: 0,
            }));
        let stats = session.get_session_stats();
        assert_eq!(stats.user_messages, 1);
        assert_eq!(stats.assistant_messages, 1);
        assert_eq!(stats.tool_calls, 1);
        assert_eq!(stats.total_messages, 2);
    }

    // ── get_last_assistant_text ───────────────────────────────────────────────

    #[test]
    fn test_get_last_assistant_text_none_when_empty() {
        let session = make_session();
        assert!(session.get_last_assistant_text().is_none());
    }

    #[test]
    fn test_get_last_assistant_text_returns_text() {
        let mut session = make_session();
        session
            .messages
            .push(AgentMessage::Assistant(AssistantMessage {
                content: vec![Content::Text {
                    text: "  Hello world  ".to_string(),
                }],
                provider: "anthropic".to_string(),
                model: "claude-opus-4-5".to_string(),
                usage: agent_core::types::Usage::default(),
                stop_reason: agent_core::types::StopReason::Stop,
                error_message: None,
                timestamp: 0,
            }));
        assert_eq!(
            session.get_last_assistant_text(),
            Some("Hello world".to_string())
        );
    }

    #[test]
    fn test_get_last_assistant_text_skips_aborted() {
        let mut session = make_session();
        // Add aborted message with no content.
        session
            .messages
            .push(AgentMessage::Assistant(AssistantMessage {
                content: vec![],
                provider: "anthropic".to_string(),
                model: "claude-opus-4-5".to_string(),
                usage: agent_core::types::Usage::default(),
                stop_reason: agent_core::types::StopReason::Aborted,
                error_message: None,
                timestamp: 0,
            }));
        assert!(session.get_last_assistant_text().is_none());
    }

    // ── is_retryable_error ────────────────────────────────────────────────────

    #[test]
    fn test_is_retryable_error_overloaded() {
        let session = make_session();
        let msg = AssistantMessage {
            content: vec![],
            provider: "anthropic".to_string(),
            model: "claude-opus-4-5".to_string(),
            usage: agent_core::types::Usage::default(),
            stop_reason: agent_core::types::StopReason::Error,
            error_message: Some("overloaded_error: too many requests".to_string()),
            timestamp: 0,
        };
        assert!(session.is_retryable_error(&msg));
    }

    #[test]
    fn test_is_retryable_error_not_for_success() {
        let session = make_session();
        let msg = AssistantMessage {
            content: vec![],
            provider: "anthropic".to_string(),
            model: "claude-opus-4-5".to_string(),
            usage: agent_core::types::Usage::default(),
            stop_reason: agent_core::types::StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };
        assert!(!session.is_retryable_error(&msg));
    }

    #[test]
    fn test_is_retryable_error_not_for_overflow() {
        let session = make_session();
        let msg = AssistantMessage {
            content: vec![],
            provider: "anthropic".to_string(),
            model: "claude-opus-4-5".to_string(),
            usage: agent_core::types::Usage::default(),
            stop_reason: agent_core::types::StopReason::Error,
            error_message: Some("prompt is too long".to_string()),
            timestamp: 0,
        };
        // Overflow errors are NOT retryable (handled by compaction).
        assert!(!session.is_retryable_error(&msg));
    }

    // ── new_session ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_new_session_clears_messages() {
        let mut session = make_session();
        session.messages.push(AgentMessage::User(UserMessage {
            content: vec![Content::Text {
                text: "test".to_string(),
            }],
            timestamp: 0,
        }));
        session.steer("pending".to_string());

        session.new_session().await;

        assert!(session.messages().is_empty());
        assert_eq!(session.pending_message_count(), 0);
    }

    // ── get_user_messages_for_forking ─────────────────────────────────────────

    #[test]
    fn test_get_user_messages_for_forking_returns_user_messages() {
        let mut session = make_session();
        session.messages.push(AgentMessage::User(UserMessage {
            content: vec![Content::Text {
                text: "First message".to_string(),
            }],
            timestamp: 0,
        }));
        session
            .messages
            .push(AgentMessage::Assistant(AssistantMessage {
                content: vec![Content::Text {
                    text: "response".to_string(),
                }],
                provider: "anthropic".to_string(),
                model: "claude-opus-4-5".to_string(),
                usage: agent_core::types::Usage::default(),
                stop_reason: agent_core::types::StopReason::Stop,
                error_message: None,
                timestamp: 0,
            }));
        session.messages.push(AgentMessage::User(UserMessage {
            content: vec![Content::Text {
                text: "Second message".to_string(),
            }],
            timestamp: 0,
        }));

        let entries = session.get_user_messages_for_forking();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "First message");
        assert_eq!(entries[1].text, "Second message");
    }

    // ── overflow recovery guard ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_overflow_recovery_attempted_only_once() {
        let mut session = make_session();
        session.model_provider = Some("anthropic".to_string());
        session.model_id = Some("claude-opus-4-5".to_string());

        let overflow_msg = AssistantMessage {
            content: vec![],
            provider: "anthropic".to_string(),
            model: "claude-opus-4-5".to_string(),
            usage: agent_core::types::Usage::default(),
            stop_reason: agent_core::types::StopReason::Error,
            error_message: Some("prompt is too long".to_string()),
            timestamp: 0,
        };

        let mut rx = session.subscribe();

        // First call — should trigger compaction.
        session.check_compaction(&overflow_msg, false).await;
        assert!(session.overflow_recovery_attempted);

        // Second call — should emit failure event, not retry.
        session.check_compaction(&overflow_msg, false).await;

        // Drain events.
        let mut got_failure = false;
        while let Ok(event) = rx.try_recv() {
            if let AgentSessionEvent::AutoCompactionEnd {
                error_message: Some(ref msg),
                ..
            } = event
            {
                if msg.contains("Context overflow recovery failed") {
                    got_failure = true;
                }
            }
        }
        assert!(got_failure, "expected overflow failure event");
    }

    // ── retry: max attempts exceeded ─────────────────────────────────────────

    #[tokio::test]
    async fn test_handle_retryable_error_max_retries() {
        // Default max_retries = 3. We call handle_retryable_error until it refuses.
        let tmp = std::env::temp_dir().join(format!("sage-test-retry-{}", std::process::id()));
        let settings_manager = Arc::new(SettingsManager::create(&tmp, &tmp));

        let mut session = AgentSession::new(AgentSessionConfig {
            cwd: tmp,
            settings_manager,
            scoped_models: vec![],
        });
        session.model_provider = Some("anthropic".to_string());
        session.model_id = Some("claude-opus-4-5".to_string());

        let err_msg = AssistantMessage {
            content: vec![],
            provider: "anthropic".to_string(),
            model: "claude-opus-4-5".to_string(),
            usage: agent_core::types::Usage::default(),
            stop_reason: agent_core::types::StopReason::Error,
            error_message: Some("503 Service Unavailable".to_string()),
            timestamp: 0,
        };

        let mut rx = session.subscribe();

        // Pre-set retry_attempt to max (3 by default) so the next call exceeds the limit
        // and returns false immediately — avoiding any real sleeps.
        session.retry_attempt = 3;

        // Now attempt 4: retry_attempt becomes 4, which is > max_retries (3) → fail.
        let retried = session.handle_retryable_error(&err_msg).await;
        assert!(!retried, "should not retry when max attempts exceeded");

        let mut got_end = false;
        while let Ok(event) = rx.try_recv() {
            if let AgentSessionEvent::AutoRetryEnd { success: false, .. } = event {
                got_end = true;
            }
        }
        assert!(got_end, "expected AutoRetryEnd with success=false");
    }

    // ── agent-session-stats.test.ts ──────────────────────────────────────────

    /// context_usage is a stub that returns None until a real model registry is wired.
    /// This test verifies that get_session_stats and get_context_usage don't panic and
    /// return consistent token totals.
    #[test]
    fn test_session_stats_token_totals_are_consistent() {
        let mut session = make_session();

        // Inject an assistant message with known token counts.
        session
            .messages
            .push(AgentMessage::Assistant(AssistantMessage {
                content: vec![Content::Text {
                    text: "response".to_string(),
                }],
                provider: "anthropic".to_string(),
                model: "claude-opus-4-5".to_string(),
                usage: agent_core::types::Usage {
                    input: 200,
                    output: 50,
                    cache_read: 10,
                    cache_write: 5,
                    total_tokens: 265,
                    cost: agent_core::types::Cost {
                        input: 0.0,
                        output: 0.0,
                        cache_read: 0.0,
                        cache_write: 0.0,
                        total: 0.0,
                    },
                },
                stop_reason: agent_core::types::StopReason::Stop,
                error_message: None,
                timestamp: 0,
            }));

        let stats = session.get_session_stats();
        assert_eq!(stats.tokens_input, 200);
        assert_eq!(stats.tokens_output, 50);
        assert_eq!(stats.tokens_cache_read, 10);
        assert_eq!(stats.tokens_cache_write, 5);
        assert_eq!(stats.tokens_total, 200 + 50 + 10 + 5);
    }

    /// Reports unknown current context usage when there's no real model registry.
    /// (Mirrors the test "reports unknown current context usage immediately after compaction"
    /// but in our Rust stub the context_usage is always None.)
    #[test]
    fn test_get_context_usage_returns_none_stub() {
        let session = make_session();
        // In the Rust port, get_context_usage() is a stub that always returns None.
        assert!(session.get_context_usage().is_none());
    }

    // ── agent-session-retry.test.ts ───────────────────────────────────────────

    #[test]
    fn test_retry_attempt_starts_at_zero() {
        let session = make_session();
        assert_eq!(session.retry_attempt(), 0);
    }

    #[test]
    fn test_is_retrying_starts_false() {
        let session = make_session();
        assert!(!session.is_retrying());
    }

    // ── agent-session-model-switch-thinking.test.ts ──────────────────────────

    #[tokio::test]
    async fn test_set_model_resets_thinking_to_off_when_not_supports_thinking() {
        // supports_thinking() is a stub returning false.
        // set_model should set thinking_level to Off (via get_thinking_level_for_model_switch).
        let mut session = make_session();
        session.thinking_level = ThinkingLevel::High;

        session
            .set_model("anthropic".to_string(), "claude-opus-4-5".to_string())
            .await
            .unwrap();

        // Since supports_thinking() returns false, thinking_level is clamped to Off.
        // (In a full implementation this would check model capabilities.)
        assert!(matches!(session.thinking_level(), ThinkingLevel::Off));
    }

    #[tokio::test]
    async fn test_cycle_model_with_scoped_models() {
        let tmp = std::env::temp_dir().join(format!("sage-test-cycle-{}", std::process::id()));
        let settings_manager = Arc::new(SettingsManager::create(&tmp, &tmp));

        let mut session = AgentSession::new(AgentSessionConfig {
            cwd: tmp,
            settings_manager,
            scoped_models: vec![
                ScopedModel {
                    model_provider: "anthropic".to_string(),
                    model_id: "claude-sonnet-4-5".to_string(),
                    thinking_level: None,
                },
                ScopedModel {
                    model_provider: "anthropic".to_string(),
                    model_id: "claude-haiku-4-5".to_string(),
                    thinking_level: None,
                },
            ],
        });
        session.model_provider = Some("anthropic".to_string());
        session.model_id = Some("claude-sonnet-4-5".to_string());

        let result = session.cycle_model(CycleDirection::Forward).await;
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.model_id, "claude-haiku-4-5");

        // Cycle again wraps around.
        let result2 = session.cycle_model(CycleDirection::Forward).await;
        assert!(result2.is_some());
        assert_eq!(result2.unwrap().model_id, "claude-sonnet-4-5");
    }

    #[tokio::test]
    async fn test_cycle_model_without_scoped_models_returns_none() {
        let mut session = make_session();
        let result = session.cycle_model(CycleDirection::Forward).await;
        assert!(result.is_none());
    }
}
