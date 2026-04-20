//! Extension system types.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/extensions/types.ts`.
//!
//! Extensions can:
//! - Subscribe to agent lifecycle events
//! - Register LLM-callable tools
//! - Register commands and CLI flags
//! - Interact with the user via UI primitives

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// Source scope / origin (from source-info.ts)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SourceScope {
    User,
    Project,
    Temporary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SourceOrigin {
    Package,
    TopLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    pub path: String,
    pub source: String,
    pub scope: SourceScope,
    pub origin: SourceOrigin,
    pub base_dir: Option<String>,
}

// ============================================================================
// UI Context
// ============================================================================

/// Where a widget is rendered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WidgetPlacement {
    AboveEditor,
    BelowEditor,
}

/// Options for extension UI dialogs.
#[derive(Debug, Clone, Default)]
pub struct ExtensionUIDialogOptions {
    /// Timeout in milliseconds. Dialog auto-dismisses with live countdown.
    pub timeout: Option<u64>,
}

/// Options for extension widgets.
#[derive(Debug, Clone, Default)]
pub struct ExtensionWidgetOptions {
    /// Where the widget is rendered. Defaults to `AboveEditor`.
    pub placement: Option<WidgetPlacement>,
}

// ============================================================================
// Context Usage
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextUsage {
    /// Estimated context tokens, or None if unknown.
    pub tokens: Option<u64>,
    pub context_window: u64,
    /// Context usage as percentage of context window, or None if tokens is unknown.
    pub percent: Option<f64>,
}

// ============================================================================
// Tool Definition
// ============================================================================

/// Rendering options for tool results.
#[derive(Debug, Clone)]
pub struct ToolRenderResultOptions {
    pub expanded: bool,
    pub is_partial: bool,
}

/// A tool definition that extensions can register.
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    /// Tool name (used in LLM tool calls).
    pub name: String,
    /// Human-readable label for UI.
    pub label: String,
    /// Description for LLM.
    pub description: String,
    /// Optional one-line snippet for the system prompt.
    pub prompt_snippet: Option<String>,
    /// Optional guideline bullets for the system prompt.
    pub prompt_guidelines: Vec<String>,
    /// JSON Schema for parameters.
    pub parameters: Value,
}

// ============================================================================
// Resource Events
// ============================================================================

/// Fired after session_start to allow extensions to provide additional resource paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcesDiscoverEvent {
    #[serde(rename = "type")]
    pub event_type: String, // "resources_discover"
    pub cwd: String,
    pub reason: ResourcesDiscoverReason,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourcesDiscoverReason {
    Startup,
    Reload,
}

#[derive(Debug, Clone, Default)]
pub struct ResourcesDiscoverResult {
    pub skill_paths: Vec<String>,
    pub prompt_paths: Vec<String>,
    pub theme_paths: Vec<String>,
}

// ============================================================================
// Session Events
// ============================================================================

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    SessionDirectory {
        cwd: String,
    },
    SessionStart,
    SessionBeforeSwitch {
        reason: SessionSwitchReason,
        target_session_file: Option<String>,
    },
    SessionSwitch {
        reason: SessionSwitchReason,
        previous_session_file: Option<String>,
    },
    SessionBeforeFork {
        entry_id: String,
    },
    SessionFork {
        previous_session_file: Option<String>,
    },
    SessionBeforeCompact {
        custom_instructions: Option<String>,
    },
    SessionCompact {
        from_extension: bool,
    },
    SessionShutdown,
    SessionBeforeTree {
        target_id: String,
    },
    SessionTree {
        new_leaf_id: Option<String>,
        old_leaf_id: Option<String>,
        from_extension: Option<bool>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionSwitchReason {
    New,
    Resume,
}

// ============================================================================
// Agent Events
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    Context {
        messages: Vec<Value>,
    },
    BeforeProviderRequest {
        payload: Value,
    },
    BeforeAgentStart {
        prompt: String,
        system_prompt: String,
    },
    AgentStart,
    AgentEnd {
        messages: Vec<Value>,
    },
    TurnStart {
        turn_index: u32,
        timestamp: i64,
    },
    TurnEnd {
        turn_index: u32,
        message: Value,
    },
    MessageStart {
        message: Value,
    },
    MessageUpdate {
        message: Value,
    },
    MessageEnd {
        message: Value,
    },
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    ToolExecutionUpdate {
        tool_call_id: String,
        tool_name: String,
        args: Value,
        partial_result: Value,
    },
    ToolExecutionEnd {
        tool_call_id: String,
        tool_name: String,
        result: Value,
        is_error: bool,
    },
}

// ============================================================================
// Model Events
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModelSelectSource {
    Set,
    Cycle,
    Restore,
}

// ============================================================================
// Input Events
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InputSource {
    Interactive,
    Rpc,
    Extension,
}

#[derive(Debug, Clone)]
pub enum InputEventResult {
    Continue,
    Transform { text: String },
    Handled,
}

// ============================================================================
// Tool Call / Result Events
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallEvent {
    #[serde(rename = "type")]
    pub event_type: String, // "tool_call"
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct ToolCallEventResult {
    pub block: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultEvent {
    #[serde(rename = "type")]
    pub event_type: String, // "tool_result"
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: HashMap<String, Value>,
    pub is_error: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ToolResultEventResult {
    pub content: Option<Vec<Value>>,
    pub details: Option<Value>,
    pub is_error: Option<bool>,
}

// ============================================================================
// User Bash Events
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserBashEvent {
    #[serde(rename = "type")]
    pub event_type: String, // "user_bash"
    pub command: String,
    pub exclude_from_context: bool,
    pub cwd: String,
}

#[derive(Debug, Clone, Default)]
pub struct UserBashEventResult {
    pub result: Option<Value>,
}

// ============================================================================
// Before Agent Start Event Result
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct BeforeAgentStartEventResult {
    pub system_prompt: Option<String>,
}

// ============================================================================
// Session Before Event Results
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct SessionBeforeSwitchResult {
    pub cancel: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SessionBeforeForkResult {
    pub cancel: bool,
    pub skip_conversation_restore: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SessionBeforeCompactResult {
    pub cancel: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SessionBeforeTreeResult {
    pub cancel: bool,
    pub custom_instructions: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SessionDirectoryResult {
    pub session_dir: Option<String>,
}

// ============================================================================
// Extension Registration Types
// ============================================================================

#[derive(Debug, Clone)]
pub struct RegisteredTool {
    pub definition: ToolDefinition,
    pub source_info: SourceInfo,
}

#[derive(Debug, Clone)]
pub struct ExtensionFlag {
    pub name: String,
    pub description: Option<String>,
    pub flag_type: FlagType,
    pub default: Option<FlagValue>,
    pub extension_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlagType {
    Boolean,
    String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FlagValue {
    Boolean(bool),
    String(String),
}

#[derive(Debug, Clone)]
pub struct RegisteredCommand {
    pub name: String,
    pub source_info: SourceInfo,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedCommand {
    pub name: String,
    pub invocation_name: String,
    pub source_info: SourceInfo,
    pub description: Option<String>,
}

// ============================================================================
// Provider Config
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelConfig {
    pub id: String,
    pub name: String,
    pub reasoning: bool,
    pub input: Vec<String>,
    pub cost: ModelCost,
    pub context_window: u64,
    pub max_tokens: u64,
    pub headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

#[derive(Debug, Clone, Default)]
pub struct ProviderConfig {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub auth_header: bool,
    pub models: Vec<ProviderModelConfig>,
}

// ============================================================================
// Extension Runtime State
// ============================================================================

#[derive(Debug, Default)]
pub struct ExtensionRuntimeState {
    pub flag_values: HashMap<String, FlagValue>,
    pub pending_provider_registrations: Vec<PendingProviderRegistration>,
}

#[derive(Debug, Clone)]
pub struct PendingProviderRegistration {
    pub name: String,
    pub config: ProviderConfig,
    pub extension_path: String,
}

// ============================================================================
// Extension Error
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionError {
    pub extension_path: String,
    pub event: String,
    pub error: String,
    pub stack: Option<String>,
}

// ============================================================================
// Tool Info
// ============================================================================

#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub source_info: SourceInfo,
}

// ============================================================================
// Loaded Extension
// ============================================================================

/// Type alias for handler function: takes event JSON and context, returns result JSON.
pub type HandlerFn =
    Box<dyn Fn(Value, &ExtensionContextSnapshot) -> Result<Option<Value>, String> + Send + Sync>;

/// A snapshot of extension context passed to handlers.
#[derive(Debug, Clone)]
pub struct ExtensionContextSnapshot {
    pub cwd: String,
    pub has_ui: bool,
}

/// A fully loaded extension with its registered items.
pub struct Extension {
    pub path: String,
    pub resolved_path: String,
    pub source_info: SourceInfo,
    pub handlers: HashMap<String, Vec<HandlerFn>>,
    pub tools: HashMap<String, RegisteredTool>,
    pub commands: HashMap<String, RegisteredCommand>,
    pub flags: HashMap<String, ExtensionFlag>,
}

/// Result of loading extensions.
pub struct LoadExtensionsResult {
    pub extensions: Vec<Extension>,
    pub errors: Vec<LoadExtensionError>,
    pub runtime: ExtensionRuntimeState,
}

#[derive(Debug, Clone)]
pub struct LoadExtensionError {
    pub path: String,
    pub error: String,
}
