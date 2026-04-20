//! RPC protocol types for headless operation.
//!
//! Translated from pi-mono `packages/coding-agent/src/modes/rpc/rpc-types.ts`.
//!
//! Commands are sent as JSON lines on stdin.
//! Responses and events are emitted as JSON lines on stdout.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// RPC Commands (stdin)
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcCommand {
    // Prompting
    Prompt {
        id: Option<String>,
        message: String,
        #[serde(default)]
        images: Vec<Value>,
        streaming_behavior: Option<StreamingBehavior>,
    },
    Steer {
        id: Option<String>,
        message: String,
        #[serde(default)]
        images: Vec<Value>,
    },
    FollowUp {
        id: Option<String>,
        message: String,
        #[serde(default)]
        images: Vec<Value>,
    },
    Abort {
        id: Option<String>,
    },
    NewSession {
        id: Option<String>,
        parent_session: Option<String>,
    },

    // State
    GetState {
        id: Option<String>,
    },

    // Model
    SetModel {
        id: Option<String>,
        provider: String,
        model_id: String,
    },
    CycleModel {
        id: Option<String>,
    },
    GetAvailableModels {
        id: Option<String>,
    },

    // Thinking
    SetThinkingLevel {
        id: Option<String>,
        level: ThinkingLevel,
    },
    CycleThinkingLevel {
        id: Option<String>,
    },

    // Queue modes
    SetSteeringMode {
        id: Option<String>,
        mode: QueueMode,
    },
    SetFollowUpMode {
        id: Option<String>,
        mode: QueueMode,
    },

    // Compaction
    Compact {
        id: Option<String>,
        custom_instructions: Option<String>,
    },
    SetAutoCompaction {
        id: Option<String>,
        enabled: bool,
    },

    // Retry
    SetAutoRetry {
        id: Option<String>,
        enabled: bool,
    },
    AbortRetry {
        id: Option<String>,
    },

    // Bash
    Bash {
        id: Option<String>,
        command: String,
    },
    AbortBash {
        id: Option<String>,
    },

    // Session
    GetSessionStats {
        id: Option<String>,
    },
    ExportHtml {
        id: Option<String>,
        output_path: Option<String>,
    },
    SwitchSession {
        id: Option<String>,
        session_path: String,
    },
    Fork {
        id: Option<String>,
        entry_id: String,
    },
    GetForkMessages {
        id: Option<String>,
    },
    GetLastAssistantText {
        id: Option<String>,
    },
    SetSessionName {
        id: Option<String>,
        name: String,
    },

    // Messages
    GetMessages {
        id: Option<String>,
    },

    // Commands
    GetCommands {
        id: Option<String>,
    },
}

impl RpcCommand {
    /// Extract the `id` field from any variant.
    pub fn id(&self) -> Option<&str> {
        match self {
            RpcCommand::Prompt { id, .. } => id.as_deref(),
            RpcCommand::Steer { id, .. } => id.as_deref(),
            RpcCommand::FollowUp { id, .. } => id.as_deref(),
            RpcCommand::Abort { id } => id.as_deref(),
            RpcCommand::NewSession { id, .. } => id.as_deref(),
            RpcCommand::GetState { id } => id.as_deref(),
            RpcCommand::SetModel { id, .. } => id.as_deref(),
            RpcCommand::CycleModel { id } => id.as_deref(),
            RpcCommand::GetAvailableModels { id } => id.as_deref(),
            RpcCommand::SetThinkingLevel { id, .. } => id.as_deref(),
            RpcCommand::CycleThinkingLevel { id } => id.as_deref(),
            RpcCommand::SetSteeringMode { id, .. } => id.as_deref(),
            RpcCommand::SetFollowUpMode { id, .. } => id.as_deref(),
            RpcCommand::Compact { id, .. } => id.as_deref(),
            RpcCommand::SetAutoCompaction { id, .. } => id.as_deref(),
            RpcCommand::SetAutoRetry { id, .. } => id.as_deref(),
            RpcCommand::AbortRetry { id } => id.as_deref(),
            RpcCommand::Bash { id, .. } => id.as_deref(),
            RpcCommand::AbortBash { id } => id.as_deref(),
            RpcCommand::GetSessionStats { id } => id.as_deref(),
            RpcCommand::ExportHtml { id, .. } => id.as_deref(),
            RpcCommand::SwitchSession { id, .. } => id.as_deref(),
            RpcCommand::Fork { id, .. } => id.as_deref(),
            RpcCommand::GetForkMessages { id } => id.as_deref(),
            RpcCommand::GetLastAssistantText { id } => id.as_deref(),
            RpcCommand::SetSessionName { id, .. } => id.as_deref(),
            RpcCommand::GetMessages { id } => id.as_deref(),
            RpcCommand::GetCommands { id } => id.as_deref(),
        }
    }

    /// Return the command type string (for error messages).
    pub fn type_name(&self) -> &'static str {
        match self {
            RpcCommand::Prompt { .. } => "prompt",
            RpcCommand::Steer { .. } => "steer",
            RpcCommand::FollowUp { .. } => "follow_up",
            RpcCommand::Abort { .. } => "abort",
            RpcCommand::NewSession { .. } => "new_session",
            RpcCommand::GetState { .. } => "get_state",
            RpcCommand::SetModel { .. } => "set_model",
            RpcCommand::CycleModel { .. } => "cycle_model",
            RpcCommand::GetAvailableModels { .. } => "get_available_models",
            RpcCommand::SetThinkingLevel { .. } => "set_thinking_level",
            RpcCommand::CycleThinkingLevel { .. } => "cycle_thinking_level",
            RpcCommand::SetSteeringMode { .. } => "set_steering_mode",
            RpcCommand::SetFollowUpMode { .. } => "set_follow_up_mode",
            RpcCommand::Compact { .. } => "compact",
            RpcCommand::SetAutoCompaction { .. } => "set_auto_compaction",
            RpcCommand::SetAutoRetry { .. } => "set_auto_retry",
            RpcCommand::AbortRetry { .. } => "abort_retry",
            RpcCommand::Bash { .. } => "bash",
            RpcCommand::AbortBash { .. } => "abort_bash",
            RpcCommand::GetSessionStats { .. } => "get_session_stats",
            RpcCommand::ExportHtml { .. } => "export_html",
            RpcCommand::SwitchSession { .. } => "switch_session",
            RpcCommand::Fork { .. } => "fork",
            RpcCommand::GetForkMessages { .. } => "get_fork_messages",
            RpcCommand::GetLastAssistantText { .. } => "get_last_assistant_text",
            RpcCommand::SetSessionName { .. } => "set_session_name",
            RpcCommand::GetMessages { .. } => "get_messages",
            RpcCommand::GetCommands { .. } => "get_commands",
        }
    }
}

// ============================================================================
// Supporting enums
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Off,
    Low,
    Medium,
    High,
    Max,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum StreamingBehavior {
    Steer,
    FollowUp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum QueueMode {
    All,
    OneAtATime,
}

// ============================================================================
// RPC Slash Command (for get_commands response)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcSlashCommand {
    pub name: String,
    pub description: Option<String>,
    pub source: SlashCommandSource,
    pub source_info: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SlashCommandSource {
    Extension,
    Prompt,
    Skill,
}

// ============================================================================
// RPC Session State
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcSessionState {
    pub model: Option<Value>,
    pub thinking_level: ThinkingLevel,
    pub is_streaming: bool,
    pub is_compacting: bool,
    pub steering_mode: QueueMode,
    pub follow_up_mode: QueueMode,
    pub session_file: Option<String>,
    pub session_id: String,
    pub session_name: Option<String>,
    pub auto_compaction_enabled: bool,
    pub message_count: usize,
    pub pending_message_count: usize,
}

// ============================================================================
// RPC Responses (stdout)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub response_type: String, // always "response"
    pub command: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl RpcResponse {
    pub fn ok(id: Option<String>, command: impl Into<String>, data: Option<Value>) -> Self {
        RpcResponse {
            id,
            response_type: "response".to_string(),
            command: command.into(),
            success: true,
            data,
            error: None,
        }
    }

    pub fn err(id: Option<String>, command: impl Into<String>, message: impl Into<String>) -> Self {
        RpcResponse {
            id,
            response_type: "response".to_string(),
            command: command.into(),
            success: false,
            data: None,
            error: Some(message.into()),
        }
    }
}

// ============================================================================
// Extension UI Requests (stdout) — for RPC extension UI bridge
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcExtensionUIRequest {
    #[serde(rename = "extension_ui_request")]
    Select {
        id: String,
        method: String, // "select"
        title: String,
        options: Vec<String>,
        timeout: Option<u64>,
    },
    #[serde(rename = "extension_ui_request")]
    Confirm {
        id: String,
        method: String, // "confirm"
        title: String,
        message: String,
        timeout: Option<u64>,
    },
    #[serde(rename = "extension_ui_request")]
    Input {
        id: String,
        method: String, // "input"
        title: String,
        placeholder: Option<String>,
        timeout: Option<u64>,
    },
    #[serde(rename = "extension_ui_request")]
    Editor {
        id: String,
        method: String, // "editor"
        title: String,
        prefill: Option<String>,
    },
    #[serde(rename = "extension_ui_request")]
    Notify {
        id: String,
        method: String, // "notify"
        message: String,
        notify_type: Option<String>,
    },
    #[serde(rename = "extension_ui_request")]
    SetStatus {
        id: String,
        method: String, // "setStatus"
        status_key: String,
        status_text: Option<String>,
    },
    #[serde(rename = "extension_ui_request")]
    SetWidget {
        id: String,
        method: String, // "setWidget"
        widget_key: String,
        widget_lines: Option<Vec<String>>,
        widget_placement: Option<String>,
    },
    #[serde(rename = "extension_ui_request")]
    SetTitle {
        id: String,
        method: String, // "setTitle"
        title: String,
    },
    #[serde(rename = "extension_ui_request")]
    SetEditorText {
        id: String,
        method: String, // "set_editor_text"
        text: String,
    },
}

// ============================================================================
// Extension UI Responses (stdin)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcExtensionUIResponse {
    Value {
        #[serde(rename = "type")]
        response_type: String, // "extension_ui_response"
        id: String,
        value: String,
    },
    Confirmed {
        #[serde(rename = "type")]
        response_type: String,
        id: String,
        confirmed: bool,
    },
    Cancelled {
        #[serde(rename = "type")]
        response_type: String,
        id: String,
        cancelled: bool,
    },
}

impl RpcExtensionUIResponse {
    pub fn id(&self) -> &str {
        match self {
            RpcExtensionUIResponse::Value { id, .. } => id,
            RpcExtensionUIResponse::Confirmed { id, .. } => id,
            RpcExtensionUIResponse::Cancelled { id, .. } => id,
        }
    }
}
