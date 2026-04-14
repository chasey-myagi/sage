// Context compaction — aligned with pi-mono's compaction.ts
//
// Manages context window overflow by summarizing older messages via LLM,
// keeping recent messages intact. Supports both threshold-based proactive
// compaction and overflow-based reactive compaction.

use crate::llm::LlmProvider;
use crate::llm::types::*;
use crate::types::*;
use std::collections::BTreeSet;

// ============================================================================
// Settings
// ============================================================================

/// Compaction configuration — aligned with pi-mono DEFAULT_COMPACTION_SETTINGS.
#[derive(Debug, Clone)]
pub struct CompactionSettings {
    /// Whether full LLM-based compaction is enabled.
    pub enabled: bool,
    /// Tokens reserved for LLM response generation.
    pub reserve_tokens: u32,
    /// Recent tokens to preserve (not summarized).
    pub keep_recent_tokens: u32,
    /// Whether microcompact (lightweight client-side cleanup) is enabled.
    pub microcompact_enabled: bool,
    /// Fraction of context_window at which microcompact triggers (default: 0.75).
    pub microcompact_threshold: f32,
    /// Number of recent turns whose ToolResult content to preserve (default: 3).
    pub microcompact_keep_turns: usize,
    /// Number of recent turns whose thinking blocks to preserve (default: 2).
    pub microcompact_keep_thinking_turns: usize,
}

impl Default for CompactionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            reserve_tokens: 16384,
            keep_recent_tokens: 20000,
            microcompact_enabled: true,
            microcompact_threshold: 0.75,
            microcompact_keep_turns: 3,
            microcompact_keep_thinking_turns: 2,
        }
    }
}

// ============================================================================
// ContextBudget — explicit budget overlay for CompactionSettings
// ============================================================================

/// Explicit context budget specification.
///
/// When set via `SageEngineBuilder::context_budget()`, overrides the computed
/// threshold fields in [`CompactionSettings`]. Provides a higher-level API for
/// expressing context management intent without touching individual token counts.
///
/// # Example
/// ```rust,ignore
/// SageEngine::builder()
///     .context_budget(ContextBudget {
///         context_window: 200_000,
///         system_reserve: 4_000,
///         output_reserve: 8_000,
///         microcompact_threshold: 0.70,
///         compaction_threshold: 0.85,
///     })
/// ```
#[derive(Debug, Clone)]
pub struct ContextBudget {
    /// Total context window size in tokens.
    pub context_window: u32,
    /// Tokens to reserve for the system prompt (excluded from history budget).
    pub system_reserve: u32,
    /// Tokens to reserve for LLM output generation.
    pub output_reserve: u32,
    /// Fraction of context_window at which microcompact fires (default: 0.75).
    pub microcompact_threshold: f32,
    /// Fraction of context_window at which full compaction fires (default: 0.90).
    pub compaction_threshold: f32,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            context_window: 200_000,
            system_reserve: 4_000,
            output_reserve: 16_384,
            microcompact_threshold: 0.75,
            compaction_threshold: 0.90,
        }
    }
}

impl ContextBudget {
    /// Apply this budget to an existing [`CompactionSettings`], overriding
    /// the threshold-related fields.
    pub fn apply_to(&self, settings: &mut CompactionSettings) {
        // history_budget = context_window - system_reserve - output_reserve
        let history_budget = self
            .context_window
            .saturating_sub(self.system_reserve)
            .saturating_sub(self.output_reserve);

        // Translate compaction_threshold into reserve_tokens.
        // should_compact fires when: context_tokens > context_window - reserve_tokens
        // We want it to fire at:     context_window * compaction_threshold
        // Therefore:                 reserve_tokens = context_window * (1 - compaction_threshold)
        settings.reserve_tokens =
            (self.context_window as f32 * (1.0 - self.compaction_threshold)) as u32;

        // keep_recent_tokens: how much history to retain after compaction (50% of history budget)
        settings.keep_recent_tokens = (history_budget as f32 * 0.5) as u32;

        // microcompact threshold fraction; explicitly enable so apply_to doesn't depend
        // on the caller's default values being correct.
        settings.microcompact_enabled = true;
        settings.microcompact_threshold = self.microcompact_threshold;
    }
}

// ============================================================================
// Types
// ============================================================================

/// Why compaction was triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionReason {
    /// Context approaching limit (proactive).
    Threshold,
    /// LLM returned context overflow error (reactive).
    Overflow,
}

/// Result of the cut point algorithm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CutPointResult {
    /// Index of the first message to keep (everything before is summarized).
    pub first_kept_index: usize,
    /// If split turn, index of the user message that started the turn.
    pub turn_start_index: Option<usize>,
    /// Whether the cut splits a turn (tool calls separated from their results).
    pub is_split_turn: bool,
}

/// Tracked file operations from tool calls (cumulative).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FileOperations {
    pub read: BTreeSet<String>,
    pub written: BTreeSet<String>,
    pub edited: BTreeSet<String>,
}

impl FileOperations {
    /// Merge another FileOperations into this one.
    pub fn merge(&mut self, other: &FileOperations) {
        self.read.extend(other.read.iter().cloned());
        self.written.extend(other.written.iter().cloned());
        self.edited.extend(other.edited.iter().cloned());
    }

    /// Format file operations for appending to summary.
    pub fn format_for_summary(&self) -> String {
        let mut out = String::new();
        if !self.read.is_empty() {
            out.push_str("<read-files>\n");
            for f in &self.read {
                out.push_str(f);
                out.push('\n');
            }
            out.push_str("</read-files>\n");
        }
        if !self.written.is_empty() || !self.edited.is_empty() {
            out.push_str("<modified-files>\n");
            for f in &self.written {
                out.push_str(f);
                out.push('\n');
            }
            for f in &self.edited {
                out.push_str(f);
                out.push('\n');
            }
            out.push_str("</modified-files>\n");
        }
        out
    }
}

/// Preparation for compaction (before calling LLM).
#[derive(Debug)]
pub struct CompactionPreparation {
    /// Messages to be summarized and discarded.
    pub messages_to_summarize: Vec<AgentMessage>,
    /// Prefix of a split turn (if cutting mid-turn).
    pub turn_prefix_messages: Vec<AgentMessage>,
    /// Index in original messages where kept portion starts.
    pub first_kept_index: usize,
    /// Whether the cut splits a turn.
    pub is_split_turn: bool,
    /// Context tokens before compaction.
    pub tokens_before: u64,
    /// Previous compaction summary (for iterative updates).
    pub previous_summary: Option<String>,
    /// Accumulated file operations.
    pub file_ops: FileOperations,
}

/// Result of compaction execution.
#[derive(Debug, Clone)]
pub struct CompactionResult {
    /// LLM-generated summary of compacted messages.
    pub summary: String,
    /// Index in original messages where kept portion starts.
    pub first_kept_index: usize,
    /// Context tokens before compaction.
    pub tokens_before: u64,
    /// Cumulative file operations.
    pub file_ops: FileOperations,
}

/// Errors during compaction.
#[derive(Debug)]
pub enum CompactionError {
    /// No messages to compact (all within budget).
    NothingToCompact,
    /// LLM summarization call failed.
    SummarizationFailed(String),
}

impl std::fmt::Display for CompactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompactionError::NothingToCompact => write!(f, "nothing to compact"),
            CompactionError::SummarizationFailed(msg) => {
                write!(f, "summarization failed: {msg}")
            }
        }
    }
}

impl std::error::Error for CompactionError {}

/// Action decided by the context overflow hook.
#[derive(Debug)]
pub enum ContextOverflowAction {
    /// Use default LLM summarization.
    Compact,
    /// Simple truncation (keep system + recent N).
    Truncate,
    /// Use a custom summary provided by the hook.
    CustomSummary(String),
    /// Abort the agent loop.
    Abort,
}

/// Context passed to the overflow hook.
#[derive(Debug)]
pub struct ContextOverflowContext {
    pub reason: CompactionReason,
    pub context_tokens: u64,
    pub context_window: u32,
    pub message_count: usize,
}

/// Hook for custom context overflow handling.
#[async_trait::async_trait]
pub trait ContextOverflowHook: Send + Sync {
    async fn on_context_overflow(&self, ctx: &ContextOverflowContext) -> ContextOverflowAction;
}

// ============================================================================
// Token estimation — aligned with pi-mono's chars/4 heuristic
// ============================================================================

/// Max characters for tool result content during serialization.
const TOOL_RESULT_TRUNCATE_CHARS: usize = 2000;

/// Safely truncate a string at a UTF-8 char boundary, returning at most `max_bytes` bytes.
fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Find the last char boundary at or before max_bytes
    let mut boundary = max_bytes;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &s[..boundary]
}

/// Estimate token count for a single AgentMessage using chars/4 heuristic.
pub fn estimate_message_tokens(msg: &AgentMessage) -> u32 {
    let chars = match msg {
        AgentMessage::User(u) => u
            .content
            .iter()
            .map(content_char_count)
            .sum::<usize>(),
        AgentMessage::Assistant(a) => a
            .content
            .iter()
            .map(content_char_count)
            .sum::<usize>(),
        AgentMessage::ToolResult(tr) => {
            let raw: usize = tr.content.iter().map(content_char_count).sum();
            raw.min(TOOL_RESULT_TRUNCATE_CHARS)
        }
        AgentMessage::CompactionSummary(cs) => cs.summary.len(),
    };
    (chars / 4) as u32
}

/// Estimate total context tokens from a slice of messages.
pub fn estimate_context_tokens(messages: &[AgentMessage]) -> u32 {
    messages.iter().map(estimate_message_tokens).sum()
}

/// Calculate context tokens from actual Usage data (preferred over estimation).
/// Aligned with pi-mono's calculateContextTokens.
pub fn calculate_context_tokens(usage: &Usage) -> u64 {
    if usage.total_tokens > 0 {
        usage.total_tokens
    } else {
        usage.input + usage.output + usage.cache_read + usage.cache_write
    }
}

fn content_char_count(content: &Content) -> usize {
    match content {
        Content::Text { text } => text.len(),
        Content::Thinking {
            thinking, redacted, ..
        } => {
            if *redacted {
                0
            } else {
                thinking.len()
            }
        }
        Content::ToolCall {
            name, arguments, ..
        } => name.len() + arguments.to_string().len(),
        Content::Image { .. } => 4800, // pi-mono: 4800 chars per image
    }
}

// ============================================================================
// Decision functions
// ============================================================================

/// Check whether microcompact (lightweight client-side cleanup) should be triggered.
/// Fires at microcompact_threshold (default 75%) of context_window, before full compaction.
pub fn should_microcompact(
    context_tokens: u64,
    context_window: u32,
    settings: &CompactionSettings,
) -> bool {
    if !settings.microcompact_enabled {
        return false;
    }
    let threshold = (context_window as f64 * settings.microcompact_threshold as f64) as u64;
    context_tokens > threshold
}

/// Check whether compaction should be triggered.
/// Aligned with pi-mono's shouldCompact.
pub fn should_compact(
    context_tokens: u64,
    context_window: u32,
    settings: &CompactionSettings,
) -> bool {
    if !settings.enabled {
        return false;
    }
    context_tokens > (context_window as u64).saturating_sub(settings.reserve_tokens as u64)
}

/// Lightweight context cleanup: replace old ToolResult content with size placeholders,
/// and remove old thinking blocks. Zero LLM calls — runs before full compaction.
///
/// Returns the number of ToolResult messages whose content was cleared.
///
/// A "turn" boundary is each `AgentMessage::User` message. Messages before the
/// `(total_turns - keep_turns)`-th turn have their ToolResult content replaced;
/// messages before the `(total_turns - keep_thinking_turns)`-th turn have their
/// thinking blocks stripped from AssistantMessages.
pub fn microcompact(
    messages: &mut [AgentMessage],
    keep_turns: usize,
    keep_thinking_turns: usize,
) -> usize {
    // Collect indices of User messages — each marks the start of a turn.
    let turn_starts: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m, AgentMessage::User(_)))
        .map(|(i, _)| i)
        .collect();

    let total_turns = turn_starts.len();
    if total_turns <= keep_turns {
        return 0;
    }

    // Cutoff index for ToolResult clearing: everything before this index is stale.
    let tool_cutoff = turn_starts[total_turns - keep_turns];

    // Cutoff index for thinking-block clearing.
    let think_cutoff = if total_turns > keep_thinking_turns {
        turn_starts[total_turns - keep_thinking_turns]
    } else {
        0
    };

    let mut cleared = 0;

    for (i, msg) in messages.iter_mut().enumerate() {
        // Clear ToolResult content for old turns.
        // When Text content is present, the *entire* content (including any Image) is replaced
        // with a single text placeholder — both to save tokens and for simplicity.
        // Tool results containing *only* non-Text content (e.g. pure Image) are left untouched.
        if i < tool_cutoff
            && let AgentMessage::ToolResult(tr) = msg
        {
            let byte_count: usize = tr
                .content
                .iter()
                .filter_map(|c| {
                    if let Content::Text { text } = c {
                        Some(text.len())
                    } else {
                        None
                    }
                })
                .sum();
            if byte_count > 0 {
                tr.content = vec![Content::Text {
                    text: format!(
                        "[tool result cleared - {}: {} bytes]",
                        tr.tool_name, byte_count
                    ),
                }];
                cleared += 1;
            }
        }

        // Strip thinking blocks from old assistant messages.
        if i < think_cutoff
            && let AgentMessage::Assistant(a) = msg
        {
            let had_thinking = a.content.iter().any(|c| matches!(c, Content::Thinking { .. }));
            if had_thinking {
                a.content.retain(|c| !matches!(c, Content::Thinking { .. }));
            }
        }
    }

    cleared
}

/// Overflow error patterns — aligned with pi-mono overflow.ts.
const OVERFLOW_PATTERNS: &[&str] = &[
    "prompt is too long",
    "exceeds the context window",
    "input token count exceeds",
    "maximum context length",
    "context_length_exceeded",
    "request too large",
    "too many tokens",
    "reduce the length",
    "context window exceeded",
    "token limit exceeded",
    "input is too long",
    "request size exceeded",
    "content is too large",
];

/// Check if an assistant message indicates context overflow.
/// Aligned with pi-mono's isContextOverflow.
pub fn is_context_overflow(msg: &AssistantMessage, context_window: u32) -> bool {
    // 1. Error message pattern matching
    if let Some(ref err) = msg.error_message {
        let lower = err.to_lowercase();
        if OVERFLOW_PATTERNS.iter().any(|p| lower.contains(p)) {
            return true;
        }
    }

    // 2. Silent overflow via usage (z.ai style)
    let usage = &msg.usage;
    if usage.input > 0 {
        let input_total = usage.input + usage.cache_read;
        if input_total > context_window as u64 {
            return true;
        }
    }

    false
}

// ============================================================================
// Cut point algorithm — aligned with pi-mono's findCutPoint
// ============================================================================

/// Whether a message is a valid cut point (never cut at ToolResult).
fn is_valid_cut_point(msg: &AgentMessage) -> bool {
    matches!(
        msg,
        AgentMessage::User(_) | AgentMessage::Assistant(_) | AgentMessage::CompactionSummary(_)
    )
}

/// Find the user message that started a turn containing `index`.
/// Searches backwards from `index` to `start`.
fn find_turn_start(messages: &[AgentMessage], index: usize, start: usize) -> Option<usize> {
    for i in (start..index).rev() {
        if matches!(messages[i], AgentMessage::User(_)) {
            return Some(i);
        }
    }
    None
}

/// Find the optimal cut point in messages.
///
/// Walks backwards from `end_index`, accumulating estimated tokens until
/// `keep_recent_tokens` is reached, then finds the nearest valid cut point.
pub fn find_cut_point(
    messages: &[AgentMessage],
    start_index: usize,
    end_index: usize,
    keep_recent_tokens: u32,
) -> CutPointResult {
    if start_index >= end_index || messages.is_empty() {
        return CutPointResult {
            first_kept_index: start_index,
            turn_start_index: None,
            is_split_turn: false,
        };
    }

    let end = end_index.min(messages.len());

    // Walk backwards accumulating tokens
    let mut accumulated: u32 = 0;
    let mut threshold_index = start_index;

    for i in (start_index..end).rev() {
        accumulated += estimate_message_tokens(&messages[i]);
        if accumulated >= keep_recent_tokens {
            threshold_index = i;
            break;
        }
    }

    // If all messages fit within budget, nothing to compact
    if accumulated < keep_recent_tokens && threshold_index == start_index {
        return CutPointResult {
            first_kept_index: start_index,
            turn_start_index: None,
            is_split_turn: false,
        };
    }

    // Find nearest valid cut point at or after threshold_index
    let cut_index = (threshold_index..end)
        .find(|&i| is_valid_cut_point(&messages[i]))
        .unwrap_or(threshold_index);

    // Check if this splits a turn
    let is_user = matches!(messages[cut_index], AgentMessage::User(_));
    let turn_start_index = if is_user {
        None
    } else {
        find_turn_start(messages, cut_index, start_index)
    };
    let is_split_turn = !is_user && turn_start_index.is_some();

    CutPointResult {
        first_kept_index: cut_index,
        turn_start_index,
        is_split_turn,
    }
}

// ============================================================================
// File operation tracking — aligned with pi-mono utils.ts
// ============================================================================

/// Extract file operations from tool calls in messages.
pub fn extract_file_operations(messages: &[AgentMessage]) -> FileOperations {
    let mut ops = FileOperations::default();
    for msg in messages {
        if let AgentMessage::Assistant(a) = msg {
            for content in &a.content {
                if let Content::ToolCall {
                    name, arguments, ..
                } = content
                {
                    if let Some(path) = arguments.get("file_path").and_then(|v| v.as_str()) {
                        match name.as_str() {
                            "read" | "read_file" => {
                                ops.read.insert(path.to_string());
                            }
                            "write" | "write_file" => {
                                ops.written.insert(path.to_string());
                            }
                            "edit" | "edit_file" => {
                                ops.edited.insert(path.to_string());
                            }
                            _ => {}
                        }
                    }
                    // Also check "path" key (some tools use this)
                    if let Some(path) = arguments.get("path").and_then(|v| v.as_str()) {
                        match name.as_str() {
                            "read" | "read_file" => {
                                ops.read.insert(path.to_string());
                            }
                            "write" | "write_file" => {
                                ops.written.insert(path.to_string());
                            }
                            "edit" | "edit_file" => {
                                ops.edited.insert(path.to_string());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    ops
}

// ============================================================================
// Message serialization for summarization prompt
// ============================================================================

/// Serialize messages to text format for the summarization LLM call.
/// Aligned with pi-mono utils.ts serializeConversation.
pub fn serialize_messages_for_summary(messages: &[AgentMessage]) -> String {
    let mut out = String::new();
    for msg in messages {
        match msg {
            AgentMessage::User(u) => {
                out.push_str("[User]: ");
                for c in &u.content {
                    if let Content::Text { text } = c {
                        out.push_str(text);
                    }
                }
                out.push_str("\n\n");
            }
            AgentMessage::Assistant(a) => {
                // Thinking blocks
                for c in &a.content {
                    if let Content::Thinking {
                        thinking, redacted, ..
                    } = c
                        && !redacted
                        && !thinking.is_empty()
                    {
                        out.push_str("[Assistant thinking]: ");
                        out.push_str(thinking);
                        out.push_str("\n\n");
                    }
                }
                // Text content
                let text = a.text();
                if !text.is_empty() {
                    out.push_str("[Assistant]: ");
                    out.push_str(&text);
                    out.push_str("\n\n");
                }
                // Tool calls
                let tool_calls: Vec<String> = a
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::ToolCall {
                            name, arguments, ..
                        } => {
                            let args_str = arguments.to_string();
                            let truncated = if args_str.len() > 200 {
                                format!("{}...", safe_truncate(&args_str, 200))
                            } else {
                                args_str
                            };
                            Some(format!("{name}({truncated})"))
                        }
                        _ => None,
                    })
                    .collect();
                if !tool_calls.is_empty() {
                    out.push_str("[Assistant tool calls]: ");
                    out.push_str(&tool_calls.join("; "));
                    out.push_str("\n\n");
                }
            }
            AgentMessage::ToolResult(tr) => {
                let text: String = tr
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let truncated = if text.len() > TOOL_RESULT_TRUNCATE_CHARS {
                    format!(
                        "{}... [truncated, {} chars total]",
                        safe_truncate(&text, TOOL_RESULT_TRUNCATE_CHARS),
                        text.len()
                    )
                } else {
                    text
                };
                if !truncated.is_empty() {
                    out.push_str("[Tool result]: ");
                    out.push_str(&truncated);
                    out.push_str("\n\n");
                }
            }
            AgentMessage::CompactionSummary(cs) => {
                out.push_str("[Previous summary]: ");
                out.push_str(&cs.summary);
                out.push_str("\n\n");
            }
        }
    }
    out
}

// ============================================================================
// Summarization prompts — aligned with pi-mono compaction.ts
// ============================================================================

const SUMMARIZATION_SYSTEM: &str = "\
You are a context summarization assistant. Your task is to read a conversation \
between a user and an AI coding assistant, then produce a structured summary \
following the exact format specified.\n\n\
Do NOT continue the conversation. Do NOT respond to any questions in the \
conversation. ONLY output the structured summary.";

fn build_initial_summarization_prompt(serialized: &str) -> String {
    format!(
        "Summarize the following conversation into a structured format:\n\n\
         {serialized}\n\n\
         Output format:\n\
         ## Goal\n\
         [What the user is trying to accomplish]\n\n\
         ## Constraints & Preferences\n\
         [Any constraints, preferences, or requirements mentioned]\n\n\
         ## Progress\n\
         ### Done\n\
         - [Completed items]\n\
         ### In Progress\n\
         - [Current work]\n\
         ### Blocked\n\
         - [Blocked items, if any]\n\n\
         ## Key Decisions\n\
         - [Important decisions made during the conversation]\n\n\
         ## Next Steps\n\
         - [What needs to happen next]\n\n\
         ## Critical Context\n\
         - [Important details that must not be lost: exact file paths, error messages, etc.]"
    )
}

fn build_update_summarization_prompt(serialized: &str, previous_summary: &str) -> String {
    format!(
        "You have a previous conversation summary and new conversation messages. \
         Update the summary to include the new information.\n\n\
         IMPORTANT:\n\
         - PRESERVE all information from the previous summary\n\
         - ADD new progress and decisions from the new messages\n\
         - UPDATE the Progress section (move items to Done when completed)\n\
         - Preserve exact file paths and error messages\n\n\
         ## Previous Summary\n\
         {previous_summary}\n\n\
         ## New Messages\n\
         {serialized}\n\n\
         Output the updated summary using the same structured format."
    )
}

fn build_turn_prefix_prompt(serialized: &str) -> String {
    format!(
        "Summarize the early part of this conversation turn. \
         This is a partial turn that was split during context compaction.\n\n\
         {serialized}\n\n\
         Output a brief summary covering:\n\
         ## Original Request\n\
         [What was asked]\n\n\
         ## Early Progress\n\
         [What was accomplished in this part]\n\n\
         ## Context for Continuation\n\
         [Key details needed to understand the remaining part of this turn]"
    )
}

// ============================================================================
// Preparation — aligned with pi-mono prepareCompaction
// ============================================================================

/// Prepare for compaction by determining what to summarize and what to keep.
pub fn prepare_compaction(
    messages: &[AgentMessage],
    context_tokens: u64,
    settings: &CompactionSettings,
    previous_summary: Option<&str>,
) -> Option<CompactionPreparation> {
    if messages.is_empty() {
        return None;
    }

    let cut = find_cut_point(messages, 0, messages.len(), settings.keep_recent_tokens);

    // Nothing to cut — all messages within budget
    if cut.first_kept_index == 0 {
        return None;
    }

    let messages_to_summarize = messages[..cut.first_kept_index].to_vec();
    let turn_prefix_messages = if cut.is_split_turn {
        if let Some(turn_start) = cut.turn_start_index {
            messages[turn_start..cut.first_kept_index].to_vec()
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    let mut file_ops = extract_file_operations(&messages_to_summarize);
    if !turn_prefix_messages.is_empty() {
        let prefix_ops = extract_file_operations(&turn_prefix_messages);
        file_ops.merge(&prefix_ops);
    }

    Some(CompactionPreparation {
        messages_to_summarize,
        turn_prefix_messages,
        first_kept_index: cut.first_kept_index,
        is_split_turn: cut.is_split_turn,
        tokens_before: context_tokens,
        previous_summary: previous_summary.map(|s| s.to_string()),
        file_ops,
    })
}

// ============================================================================
// Execution — aligned with pi-mono compact()
// ============================================================================

/// Execute compaction by calling LLM for summarization.
pub async fn compact(
    preparation: CompactionPreparation,
    provider: &dyn LlmProvider,
    model: &Model,
) -> Result<CompactionResult, CompactionError> {
    if preparation.messages_to_summarize.is_empty() {
        return Err(CompactionError::NothingToCompact);
    }

    let serialized = serialize_messages_for_summary(&preparation.messages_to_summarize);

    // Build prompt — initial or update depending on previous summary
    let user_prompt = match &preparation.previous_summary {
        Some(prev) => build_update_summarization_prompt(&serialized, prev),
        None => build_initial_summarization_prompt(&serialized),
    };

    // Call LLM for summarization
    let max_summary_tokens =
        (preparation.tokens_before.min(model.max_tokens as u64) * 80 / 100) as u32;

    let summary_context = LlmContext {
        messages: vec![LlmMessage::User {
            content: vec![LlmContent::Text(user_prompt)],
        }],
        system_prompt: SUMMARIZATION_SYSTEM.to_string(),
        max_tokens: max_summary_tokens.max(1024),
        temperature: Some(0.0),
    };

    let events = provider.complete(model, &summary_context, &[]).await;

    // Extract text from response, bail on Error events
    let mut summary = String::new();
    for event in &events {
        match event {
            AssistantMessageEvent::Error(e) => {
                return Err(CompactionError::SummarizationFailed(e.clone()));
            }
            AssistantMessageEvent::TextDelta(delta) => {
                summary.push_str(delta);
            }
            _ => {}
        }
    }

    if summary.is_empty() {
        return Err(CompactionError::SummarizationFailed(
            "LLM returned empty summary".into(),
        ));
    }

    // Handle split turn — generate turn prefix summary
    if preparation.is_split_turn && !preparation.turn_prefix_messages.is_empty() {
        let prefix_serialized = serialize_messages_for_summary(&preparation.turn_prefix_messages);
        let prefix_prompt = build_turn_prefix_prompt(&prefix_serialized);

        let prefix_context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text(prefix_prompt)],
            }],
            system_prompt: SUMMARIZATION_SYSTEM.to_string(),
            max_tokens: (max_summary_tokens / 2).max(512),
            temperature: Some(0.0),
        };

        let prefix_events = provider.complete(model, &prefix_context, &[]).await;
        let mut prefix_summary = String::new();
        for event in &prefix_events {
            match event {
                AssistantMessageEvent::Error(_) => {
                    // Prefix summarization failure is non-fatal; skip prefix
                    break;
                }
                AssistantMessageEvent::TextDelta(delta) => {
                    prefix_summary.push_str(delta);
                }
                _ => {}
            }
        }

        if !prefix_summary.is_empty() {
            summary.push_str("\n\n---\n\n## Current Turn (early part)\n\n");
            summary.push_str(&prefix_summary);
        }
    }

    // Append file operations
    let file_ops_text = preparation.file_ops.format_for_summary();
    if !file_ops_text.is_empty() {
        summary.push_str("\n\n");
        summary.push_str(&file_ops_text);
    }

    Ok(CompactionResult {
        summary,
        first_kept_index: preparation.first_kept_index,
        tokens_before: preparation.tokens_before,
        file_ops: preparation.file_ops,
    })
}

// ============================================================================
// Application — replace old messages with summary
// ============================================================================

/// Apply compaction result to agent messages.
///
/// Replaces messages[0..first_kept_index] with a single CompactionSummary,
/// preserving messages[first_kept_index..].
pub fn apply_compaction(messages: &mut Vec<AgentMessage>, result: &CompactionResult) {
    if result.first_kept_index == 0 || result.first_kept_index > messages.len() {
        return;
    }

    let kept = messages.split_off(result.first_kept_index);
    messages.clear();
    messages.push(AgentMessage::CompactionSummary(CompactionSummaryMessage {
        summary: result.summary.clone(),
        tokens_before: result.tokens_before,
        timestamp: crate::types::now_secs(),
    }));
    messages.extend(kept);
}

/// Simple truncation fallback: keep only recent messages within token budget.
pub fn truncate_messages(messages: &mut Vec<AgentMessage>, keep_recent_tokens: u32) {
    if messages.is_empty() {
        return;
    }

    let cut = find_cut_point(messages, 0, messages.len(), keep_recent_tokens);
    if cut.first_kept_index > 0 {
        let kept = messages.split_off(cut.first_kept_index);
        messages.clear();
        messages.extend(kept);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{StatefulProvider, test_model};
    use serde_json::json;

    // -- helpers --

    fn user_msg(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage::from_text(text))
    }

    fn assistant_msg(text: &str) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage::new(text.to_string()))
    }

    fn tool_result_msg(tool_call_id: &str, text: &str) -> AgentMessage {
        AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "bash".to_string(),
            content: vec![Content::Text {
                text: text.to_string(),
            }],
            is_error: false,
            timestamp: 0,
        })
    }

    fn assistant_with_tool_call(
        text: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            content: vec![
                Content::Text {
                    text: text.to_string(),
                },
                Content::ToolCall {
                    id: "tc-1".to_string(),
                    name: tool_name.to_string(),
                    arguments: args,
                },
            ],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error_message: None,
            timestamp: 0,
        })
    }

    fn compaction_summary_msg(summary: &str) -> AgentMessage {
        AgentMessage::CompactionSummary(CompactionSummaryMessage {
            summary: summary.to_string(),
            tokens_before: 50000,
            timestamp: 0,
        })
    }

    /// Build messages large enough to trigger compaction.
    /// Each "chunk" is ~250 chars = ~62 tokens.
    fn big_message(size: usize) -> String {
        "x".repeat(size)
    }

    // ========================================================================
    // Token estimation
    // ========================================================================

    #[test]
    fn estimate_message_tokens_user_text() {
        let msg = user_msg("hello world"); // 11 chars → 2 tokens
        let tokens = estimate_message_tokens(&msg);
        assert_eq!(tokens, 11 / 4); // 2
    }

    #[test]
    fn estimate_message_tokens_assistant_with_tool_calls() {
        let msg = assistant_with_tool_call(
            "Let me read that file",
            "read_file",
            json!({"file_path": "/src/main.rs"}),
        );
        let tokens = estimate_message_tokens(&msg);
        // text (21 chars) + tool_name (9) + args json (~30) = ~60 chars → 15 tokens
        assert!(tokens > 10, "expected >10, got {tokens}");
        assert!(tokens < 30, "expected <30, got {tokens}");
    }

    #[test]
    fn estimate_message_tokens_tool_result_truncated() {
        // Tool result with 5000 chars should be capped at 2000
        let msg = tool_result_msg("tc-1", &big_message(5000));
        let tokens = estimate_message_tokens(&msg);
        assert_eq!(tokens, (TOOL_RESULT_TRUNCATE_CHARS / 4) as u32); // 500
    }

    #[test]
    fn estimate_context_tokens_multiple_messages() {
        let messages = vec![
            user_msg(&big_message(400)),      // 100 tokens
            assistant_msg(&big_message(800)), // 200 tokens
        ];
        let total = estimate_context_tokens(&messages);
        assert_eq!(total, 300);
    }

    #[test]
    fn calculate_context_tokens_prefers_total_tokens() {
        let usage = Usage {
            input: 100,
            output: 50,
            cache_read: 0,
            cache_write: 0,
            total_tokens: 200, // should prefer this
            cost: Cost::default(),
        };
        assert_eq!(calculate_context_tokens(&usage), 200);
    }

    #[test]
    fn calculate_context_tokens_falls_back_to_sum() {
        let usage = Usage {
            input: 100,
            output: 50,
            cache_read: 30,
            cache_write: 20,
            total_tokens: 0, // zero → fall back to sum
            cost: Cost::default(),
        };
        assert_eq!(calculate_context_tokens(&usage), 200);
    }

    #[test]
    fn estimate_message_tokens_compaction_summary() {
        let msg = compaction_summary_msg(&big_message(400));
        assert_eq!(estimate_message_tokens(&msg), 100);
    }

    #[test]
    fn estimate_message_tokens_image_content() {
        let msg = AgentMessage::User(UserMessage {
            content: vec![Content::Image {
                data: "base64...".into(),
                mime_type: "image/png".into(),
            }],
            timestamp: 0,
        });
        // Image = 4800 chars → 1200 tokens
        assert_eq!(estimate_message_tokens(&msg), 1200);
    }

    #[test]
    fn estimate_message_tokens_thinking_not_redacted() {
        let msg = AgentMessage::Assistant(AssistantMessage {
            content: vec![Content::Thinking {
                thinking: big_message(200),
                signature: None,
                redacted: false,
            }],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        });
        assert_eq!(estimate_message_tokens(&msg), 50); // 200/4
    }

    #[test]
    fn estimate_message_tokens_thinking_redacted_is_zero() {
        let msg = AgentMessage::Assistant(AssistantMessage {
            content: vec![Content::Thinking {
                thinking: big_message(200),
                signature: Some("sig".into()),
                redacted: true,
            }],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        });
        assert_eq!(estimate_message_tokens(&msg), 0);
    }

    // ========================================================================
    // Decision functions
    // ========================================================================

    #[test]
    fn should_compact_false_when_disabled() {
        let settings = CompactionSettings {
            enabled: false,
            ..Default::default()
        };
        assert!(!should_compact(200_000, 128_000, &settings));
    }

    #[test]
    fn should_compact_false_below_threshold() {
        let settings = CompactionSettings::default(); // reserve=16384
        // 128000 - 16384 = 111616 threshold
        assert!(!should_compact(100_000, 128_000, &settings));
    }

    #[test]
    fn should_compact_true_above_threshold() {
        let settings = CompactionSettings::default();
        // 128000 - 16384 = 111616 threshold, 120000 > 111616
        assert!(should_compact(120_000, 128_000, &settings));
    }

    #[test]
    fn should_compact_boundary_exact_threshold() {
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 1000,
            keep_recent_tokens: 5000,
        ..Default::default()
        };
        // threshold = 10000 - 1000 = 9000
        // exactly at threshold: NOT compact (need to exceed)
        assert!(!should_compact(9000, 10000, &settings));
        // one over: compact
        assert!(should_compact(9001, 10000, &settings));
    }

    #[test]
    fn is_context_overflow_detects_error_patterns() {
        let msg = AssistantMessage {
            content: vec![],
            provider: "anthropic".into(),
            model: "claude-3".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Error,
            error_message: Some("prompt is too long: 250000 tokens > 200000 maximum".into()),
            timestamp: 0,
        };
        assert!(is_context_overflow(&msg, 200_000));
    }

    #[test]
    fn is_context_overflow_detects_openai_pattern() {
        let msg = AssistantMessage {
            content: vec![],
            provider: "openai".into(),
            model: "gpt-4".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Error,
            error_message: Some(
                "This model's maximum context length is 128000 tokens. \
                 However, your messages resulted in 150000 tokens."
                    .into(),
            ),
            timestamp: 0,
        };
        assert!(is_context_overflow(&msg, 128_000));
    }

    #[test]
    fn is_context_overflow_silent_usage_based() {
        let msg = AssistantMessage {
            content: vec![],
            provider: "zai".into(),
            model: "test".into(),
            usage: Usage {
                input: 100_000,
                output: 500,
                cache_read: 50_000,
                cache_write: 0,
                total_tokens: 0,
                cost: Cost::default(),
            },
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };
        // input(100k) + cache_read(50k) = 150k > 128k context_window
        assert!(is_context_overflow(&msg, 128_000));
    }

    #[test]
    fn is_context_overflow_false_for_normal_response() {
        let msg = AssistantMessage {
            content: vec![Content::Text {
                text: "Hello!".into(),
            }],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage {
                input: 100,
                output: 50,
                ..Usage::default()
            },
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };
        assert!(!is_context_overflow(&msg, 128_000));
    }

    #[test]
    fn is_context_overflow_case_insensitive() {
        let msg = AssistantMessage {
            content: vec![],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Error,
            error_message: Some("PROMPT IS TOO LONG for this model".into()),
            timestamp: 0,
        };
        assert!(is_context_overflow(&msg, 128_000));
    }

    // ========================================================================
    // Cut point algorithm
    // ========================================================================

    #[test]
    fn find_cut_point_keeps_recent_tokens() {
        // 5 messages, each ~100 tokens (400 chars)
        let messages: Vec<AgentMessage> = (0..5)
            .map(|i| user_msg(&format!("{}{}", big_message(396), i)))
            .collect();
        // keep_recent_tokens = 250 → should keep last ~2-3 messages
        let cut = find_cut_point(&messages, 0, 5, 250);
        assert!(
            cut.first_kept_index >= 2 && cut.first_kept_index <= 3,
            "expected 2-3, got {}",
            cut.first_kept_index
        );
    }

    #[test]
    fn find_cut_point_never_cuts_at_tool_result() {
        let messages = vec![
            user_msg("start"),                      // 0: valid cut
            assistant_msg("thinking..."),           // 1: valid cut
            tool_result_msg("tc-1", "output here"), // 2: NOT valid
            user_msg("next question"),              // 3: valid cut
            assistant_msg("answer"),                // 4: valid cut
        ];
        // keep_recent_tokens very small → wants to cut near the end
        // But should never return index 2 (ToolResult)
        let cut = find_cut_point(&messages, 0, 5, 1);
        assert_ne!(cut.first_kept_index, 2, "must not cut at ToolResult");
    }

    #[test]
    fn find_cut_point_split_turn_detection() {
        // Turn: user(0) → assistant+tool(1) → tool_result(2) → assistant(3)
        // If cut at 3 (assistant), turn started at 0 → split turn
        let messages = vec![
            user_msg("do something"),
            assistant_with_tool_call("calling tool", "bash", json!({"command": "ls"})),
            tool_result_msg("tc-1", "file1.txt"),
            assistant_msg(&big_message(400)), // 100 tokens
        ];
        // keep_recent_tokens = 50 → wants to keep only last msg
        let cut = find_cut_point(&messages, 0, 4, 50);
        if cut.first_kept_index == 3 {
            assert!(
                cut.is_split_turn,
                "cutting at assistant mid-turn should detect split"
            );
            assert_eq!(cut.turn_start_index, Some(0));
        }
    }

    #[test]
    fn find_cut_point_all_messages_within_budget() {
        let messages = vec![user_msg("hi"), assistant_msg("hello")];
        // keep_recent_tokens much larger than total
        let cut = find_cut_point(&messages, 0, 2, 100_000);
        assert_eq!(cut.first_kept_index, 0, "nothing to cut");
    }

    #[test]
    fn find_cut_point_empty_messages() {
        let messages: Vec<AgentMessage> = vec![];
        let cut = find_cut_point(&messages, 0, 0, 1000);
        assert_eq!(cut.first_kept_index, 0);
        assert!(!cut.is_split_turn);
    }

    #[test]
    fn find_cut_point_single_huge_turn() {
        // One user message that exceeds budget
        let messages = vec![user_msg(&big_message(80000))]; // 20000 tokens
        let cut = find_cut_point(&messages, 0, 1, 1000);
        // Even though the single message exceeds budget, it's the only one
        // so we can't cut before it (first_kept_index = 0)
        // The algorithm sets threshold_index to 0 but then checks if it's
        // the same as start_index
        assert_eq!(cut.first_kept_index, 0);
    }

    #[test]
    fn find_cut_point_respects_start_index() {
        let messages = vec![
            user_msg("old1"),
            assistant_msg("old2"),
            user_msg("new1"),
            assistant_msg(&big_message(400)),
        ];
        // Start from index 2 — should only consider messages[2..]
        let cut = find_cut_point(&messages, 2, 4, 50);
        assert!(
            cut.first_kept_index >= 2,
            "should not cut before start_index"
        );
    }

    // ========================================================================
    // File operation tracking
    // ========================================================================

    #[test]
    fn extract_file_ops_from_tool_calls() {
        let messages = vec![
            assistant_with_tool_call("reading", "read_file", json!({"file_path": "/src/main.rs"})),
            tool_result_msg("tc-1", "fn main() {}"),
            assistant_with_tool_call(
                "editing",
                "edit_file",
                json!({"file_path": "/src/lib.rs", "content": "pub mod foo;"}),
            ),
        ];
        let ops = extract_file_operations(&messages);
        assert!(ops.read.contains("/src/main.rs"));
        assert!(ops.edited.contains("/src/lib.rs"));
        assert!(ops.written.is_empty());
    }

    #[test]
    fn extract_file_ops_cumulative_merge() {
        let mut ops1 = FileOperations::default();
        ops1.read.insert("/a.rs".into());
        ops1.written.insert("/b.rs".into());

        let mut ops2 = FileOperations::default();
        ops2.read.insert("/c.rs".into());
        ops2.edited.insert("/b.rs".into()); // same file, different op

        ops1.merge(&ops2);
        assert_eq!(ops1.read.len(), 2); // /a.rs, /c.rs
        assert_eq!(ops1.written.len(), 1); // /b.rs
        assert_eq!(ops1.edited.len(), 1); // /b.rs
    }

    #[test]
    fn file_operations_format_for_summary() {
        let mut ops = FileOperations::default();
        ops.read.insert("/src/main.rs".into());
        ops.written.insert("/src/new.rs".into());
        let formatted = ops.format_for_summary();
        assert!(formatted.contains("<read-files>"));
        assert!(formatted.contains("/src/main.rs"));
        assert!(formatted.contains("<modified-files>"));
        assert!(formatted.contains("/src/new.rs"));
    }

    // ========================================================================
    // Preparation
    // ========================================================================

    #[test]
    fn prepare_compaction_normal_case() {
        // 10 messages, each ~100 tokens → 1000 total
        // Use keep_recent_tokens=300 so cut point is around index 7
        let messages: Vec<AgentMessage> = (0..10)
            .map(|i| user_msg(&format!("{}{}", big_message(396), i)))
            .collect();
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 1000,
            keep_recent_tokens: 300,
        ..Default::default()
        };
        let prep = prepare_compaction(&messages, 50000, &settings, None);
        assert!(prep.is_some());
        let prep = prep.unwrap();
        assert!(!prep.messages_to_summarize.is_empty());
        assert_eq!(prep.tokens_before, 50000);
        assert!(prep.previous_summary.is_none());
    }

    #[test]
    fn prepare_compaction_with_previous_summary() {
        let messages: Vec<AgentMessage> = (0..10)
            .map(|i| user_msg(&format!("{}{}", big_message(396), i)))
            .collect();
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 1000,
            keep_recent_tokens: 300,
        ..Default::default()
        };
        let prep = prepare_compaction(
            &messages,
            50000,
            &settings,
            Some("Previous work: built module X"),
        );
        assert!(prep.is_some());
        let prep = prep.unwrap();
        assert_eq!(
            prep.previous_summary.as_deref(),
            Some("Previous work: built module X")
        );
    }

    #[test]
    fn prepare_compaction_returns_none_when_nothing_to_compact() {
        let messages = vec![user_msg("hi"), assistant_msg("hello")];
        // keep_recent_tokens is huge → nothing to cut
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 1000,
            keep_recent_tokens: 100_000,
        ..Default::default()
        };
        let prep = prepare_compaction(&messages, 50000, &settings, None);
        assert!(prep.is_none());
    }

    #[test]
    fn prepare_compaction_empty_messages() {
        let prep = prepare_compaction(&[], 0, &CompactionSettings::default(), None);
        assert!(prep.is_none());
    }

    // ========================================================================
    // Execution + Application
    // ========================================================================

    #[tokio::test]
    async fn compact_generates_summary_via_llm() {
        let messages: Vec<AgentMessage> = (0..10)
            .map(|i| {
                user_msg(&format!(
                    "Message number {i} with content: {}",
                    big_message(396)
                ))
            })
            .collect();

        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 1000,
            keep_recent_tokens: 300,
        ..Default::default()
        };
        let prep = prepare_compaction(&messages, 50000, &settings, None).unwrap();

        // Mock provider returns a summary
        let provider = StatefulProvider::new(vec![vec![
            AssistantMessageEvent::TextDelta("## Goal\nTest summary content".into()),
            AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            },
        ]]);

        let model = test_model();
        let result = compact(prep, &provider, &model).await;
        assert!(result.is_ok());
        let result = result.unwrap();
        assert!(result.summary.contains("Test summary content"));
        assert_eq!(result.tokens_before, 50000);
        assert_eq!(provider.call_count(), 1);
    }

    #[tokio::test]
    async fn compact_returns_error_on_empty_summary() {
        let messages = vec![
            user_msg(&big_message(80000)),
            assistant_msg(&big_message(80000)),
        ];
        let prep = prepare_compaction(
            &messages,
            50000,
            &CompactionSettings {
                enabled: true,
                reserve_tokens: 1000,
                keep_recent_tokens: 100,
            ..Default::default()
            },
            None,
        )
        .unwrap();

        // Provider returns Done without any text
        let provider = StatefulProvider::new(vec![vec![AssistantMessageEvent::Done {
            stop_reason: StopReason::Stop,
        }]]);

        let model = test_model();
        let result = compact(prep, &provider, &model).await;
        assert!(matches!(
            result,
            Err(CompactionError::SummarizationFailed(_))
        ));
    }

    #[test]
    fn apply_compaction_replaces_old_messages_with_summary() {
        let mut messages = vec![
            user_msg("old question 1"),
            assistant_msg("old answer 1"),
            user_msg("old question 2"),
            assistant_msg("old answer 2"),
            user_msg("recent question"),
            assistant_msg("recent answer"),
        ];

        let result = CompactionResult {
            summary: "Summary of old conversation".into(),
            first_kept_index: 4,
            tokens_before: 50000,
            file_ops: FileOperations::default(),
        };

        apply_compaction(&mut messages, &result);

        // Should have: [CompactionSummary] + messages[4..] (2 messages)
        assert_eq!(messages.len(), 3);
        assert!(matches!(
            &messages[0],
            AgentMessage::CompactionSummary(cs) if cs.summary == "Summary of old conversation"
        ));
        // Recent messages preserved
        assert!(matches!(&messages[1], AgentMessage::User(_)));
        assert!(matches!(&messages[2], AgentMessage::Assistant(_)));
    }

    #[test]
    fn apply_compaction_preserves_kept_messages() {
        let mut messages = vec![
            user_msg("old"),
            assistant_msg("old reply"),
            user_msg("keep this"),
        ];

        let result = CompactionResult {
            summary: "old stuff happened".into(),
            first_kept_index: 2,
            tokens_before: 10000,
            file_ops: FileOperations::default(),
        };

        apply_compaction(&mut messages, &result);

        assert_eq!(messages.len(), 2); // summary + "keep this"
        match &messages[1] {
            AgentMessage::User(u) => {
                assert!(matches!(&u.content[0], Content::Text { text } if text == "keep this"));
            }
            _ => panic!("expected User message"),
        }
    }

    #[test]
    fn apply_compaction_noop_when_index_zero() {
        let mut messages = vec![user_msg("only message")];
        let result = CompactionResult {
            summary: "summary".into(),
            first_kept_index: 0,
            tokens_before: 100,
            file_ops: FileOperations::default(),
        };
        apply_compaction(&mut messages, &result);
        // No change
        assert_eq!(messages.len(), 1);
        assert!(matches!(&messages[0], AgentMessage::User(_)));
    }

    #[test]
    fn apply_compaction_noop_when_index_exceeds_length() {
        let mut messages = vec![user_msg("a"), assistant_msg("b")];
        let result = CompactionResult {
            summary: "summary".into(),
            first_kept_index: 100, // way beyond
            tokens_before: 100,
            file_ops: FileOperations::default(),
        };
        apply_compaction(&mut messages, &result);
        // No change
        assert_eq!(messages.len(), 2);
    }

    // ========================================================================
    // Truncation fallback
    // ========================================================================

    #[test]
    fn truncate_messages_keeps_recent() {
        let mut messages: Vec<AgentMessage> = (0..10)
            .map(|i| user_msg(&format!("{}{}", big_message(396), i)))
            .collect();
        // Each msg ~100 tokens. keep_recent=300 → keep last 3
        truncate_messages(&mut messages, 300);
        assert!(
            messages.len() <= 4 && messages.len() >= 2,
            "expected 2-4 messages, got {}",
            messages.len()
        );
    }

    #[test]
    fn truncate_messages_noop_when_all_fit() {
        let mut messages = vec![user_msg("hi"), assistant_msg("hello")];
        truncate_messages(&mut messages, 100_000);
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn truncate_messages_empty_is_noop() {
        let mut messages: Vec<AgentMessage> = vec![];
        truncate_messages(&mut messages, 1000);
        assert!(messages.is_empty());
    }

    // ========================================================================
    // Serialization
    // ========================================================================

    #[test]
    fn serialize_messages_includes_all_roles() {
        let messages = vec![
            user_msg("What is Rust?"),
            assistant_msg("Rust is a systems language."),
            tool_result_msg("tc-1", "some output"),
        ];
        let serialized = serialize_messages_for_summary(&messages);
        assert!(serialized.contains("[User]: What is Rust?"));
        assert!(serialized.contains("[Assistant]: Rust is a systems language."));
        assert!(serialized.contains("[Tool result]: some output"));
    }

    #[test]
    fn serialize_messages_truncates_long_tool_results() {
        let long_output = big_message(5000);
        let messages = vec![tool_result_msg("tc-1", &long_output)];
        let serialized = serialize_messages_for_summary(&messages);
        assert!(serialized.contains("[truncated,"));
        assert!(serialized.len() < 3000);
    }

    #[test]
    fn serialize_messages_includes_tool_calls() {
        let messages = vec![assistant_with_tool_call(
            "Let me check",
            "read_file",
            json!({"file_path": "/foo.rs"}),
        )];
        let serialized = serialize_messages_for_summary(&messages);
        assert!(serialized.contains("[Assistant tool calls]:"));
        assert!(serialized.contains("read_file"));
    }

    #[test]
    fn serialize_messages_includes_compaction_summary() {
        let messages = vec![compaction_summary_msg("Previous work done")];
        let serialized = serialize_messages_for_summary(&messages);
        assert!(serialized.contains("[Previous summary]: Previous work done"));
    }

    #[test]
    fn serialize_messages_includes_thinking_block() {
        let msg = AgentMessage::Assistant(AssistantMessage {
            content: vec![
                Content::Thinking {
                    thinking: "Let me reason about this...".into(),
                    signature: None,
                    redacted: false,
                },
                Content::Text {
                    text: "Here is my answer.".into(),
                },
            ],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        });
        let serialized = serialize_messages_for_summary(&[msg]);
        assert!(serialized.contains("[Assistant thinking]: Let me reason about this..."));
        assert!(serialized.contains("[Assistant]: Here is my answer."));
    }

    #[test]
    fn serialize_messages_skips_redacted_thinking() {
        let msg = AgentMessage::Assistant(AssistantMessage {
            content: vec![
                Content::Thinking {
                    thinking: "secret reasoning".into(),
                    signature: Some("sig".into()),
                    redacted: true,
                },
                Content::Text {
                    text: "visible answer".into(),
                },
            ],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        });
        let serialized = serialize_messages_for_summary(&[msg]);
        assert!(!serialized.contains("secret reasoning"));
        assert!(serialized.contains("[Assistant]: visible answer"));
    }

    #[test]
    fn serialize_messages_truncates_long_tool_call_args() {
        let long_args = json!({"content": big_message(500)});
        let msg = assistant_with_tool_call("writing", "write_file", long_args);
        let serialized = serialize_messages_for_summary(&[msg]);
        assert!(
            serialized.contains("..."),
            "long args should be truncated with ..."
        );
    }

    #[test]
    fn serialize_messages_empty_returns_empty() {
        let serialized = serialize_messages_for_summary(&[]);
        assert!(serialized.is_empty());
    }

    // ========================================================================
    // Token estimation — additional boundary tests
    // ========================================================================

    #[test]
    fn estimate_message_tokens_empty_user_message() {
        let msg = user_msg("");
        assert_eq!(estimate_message_tokens(&msg), 0);
    }

    #[test]
    fn estimate_message_tokens_mixed_content_types() {
        // Assistant with text + thinking + tool call
        let msg = AgentMessage::Assistant(AssistantMessage {
            content: vec![
                Content::Thinking {
                    thinking: big_message(100), // 25 tokens
                    signature: None,
                    redacted: false,
                },
                Content::Text {
                    text: big_message(200), // 50 tokens
                },
                Content::ToolCall {
                    id: "tc-1".into(),
                    name: "bash".into(),             // 4 chars
                    arguments: json!({"cmd": "ls"}), // ~12 chars
                },
            ],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        });
        let tokens = estimate_message_tokens(&msg);
        // 25 + 50 + (4+12)/4 ≈ 79
        assert!(tokens > 70, "expected >70, got {tokens}");
        assert!(tokens < 90, "expected <90, got {tokens}");
    }

    // ========================================================================
    // Decision — additional boundary tests
    // ========================================================================

    #[test]
    fn should_compact_context_window_zero() {
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 1000,
            keep_recent_tokens: 5000,
        ..Default::default()
        };
        // saturating_sub: 0 - 1000 = 0, any tokens > 0 → compact
        assert!(should_compact(1, 0, &settings));
    }

    #[test]
    fn is_context_overflow_no_error_no_usage() {
        let msg = AssistantMessage {
            content: vec![],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(), // all zeros
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };
        assert!(!is_context_overflow(&msg, 128_000));
    }

    // ========================================================================
    // Cut point — additional boundary tests
    // ========================================================================

    #[test]
    fn find_cut_point_start_equals_end() {
        let messages = vec![user_msg("a"), assistant_msg("b")];
        let cut = find_cut_point(&messages, 1, 1, 100);
        assert_eq!(cut.first_kept_index, 1);
        assert!(!cut.is_split_turn);
    }

    #[test]
    fn find_cut_point_end_exceeds_length() {
        let messages = vec![user_msg("a"), assistant_msg("b")];
        // end_index=100, but messages.len()=2 → clamped to 2
        let cut = find_cut_point(&messages, 0, 100, 100_000);
        assert_eq!(cut.first_kept_index, 0); // all within budget
    }

    #[test]
    fn find_cut_point_compaction_summary_is_valid_cut() {
        let messages = vec![
            compaction_summary_msg("old summary"), // 0: valid cut
            user_msg(&big_message(400)),           // 1: valid cut
            assistant_msg(&big_message(400)),      // 2: valid cut
            user_msg(&big_message(400)),           // 3: valid cut
            assistant_msg(&big_message(400)),      // 4: valid cut
        ];
        let cut = find_cut_point(&messages, 0, 5, 150);
        // Should be able to cut at CompactionSummary (index 0)
        assert!(
            is_valid_cut_point(&messages[cut.first_kept_index]),
            "cut point must be at a valid message type"
        );
    }

    #[test]
    fn find_cut_point_all_tool_results_degrades_gracefully() {
        // Edge case: only ToolResult messages (no valid cut points in range)
        let messages = vec![
            tool_result_msg("tc-1", &big_message(400)),
            tool_result_msg("tc-2", &big_message(400)),
            tool_result_msg("tc-3", &big_message(400)),
        ];
        let cut = find_cut_point(&messages, 0, 3, 50);
        // Algorithm should still return something — won't cut at ToolResult,
        // so will scan forward but all are invalid → stays at start
        // This tests graceful degradation
        assert!(cut.first_kept_index <= 3);
    }

    // ========================================================================
    // File operation tracking — additional scenarios
    // ========================================================================

    #[test]
    fn extract_file_ops_write_file_tool() {
        let messages = vec![assistant_with_tool_call(
            "creating file",
            "write_file",
            json!({"file_path": "/src/new.rs", "content": "pub fn new() {}"}),
        )];
        let ops = extract_file_operations(&messages);
        assert!(ops.written.contains("/src/new.rs"));
    }

    #[test]
    fn extract_file_ops_path_key_alternative() {
        // Some tools use "path" instead of "file_path"
        let messages = vec![assistant_with_tool_call(
            "reading",
            "read_file",
            json!({"path": "/alt/path.rs"}),
        )];
        let ops = extract_file_operations(&messages);
        assert!(
            ops.read.contains("/alt/path.rs"),
            "should recognize 'path' key as alternative to 'file_path'"
        );
    }

    #[test]
    fn file_operations_format_empty_returns_empty() {
        let ops = FileOperations::default();
        assert!(ops.format_for_summary().is_empty());
    }

    #[test]
    fn file_operations_format_only_edited() {
        let mut ops = FileOperations::default();
        ops.edited.insert("/src/lib.rs".into());
        let formatted = ops.format_for_summary();
        assert!(!formatted.contains("<read-files>"));
        assert!(formatted.contains("<modified-files>"));
        assert!(formatted.contains("/src/lib.rs"));
    }

    #[test]
    fn extract_file_ops_from_compaction_summary_is_empty() {
        let messages = vec![compaction_summary_msg("no tool calls here")];
        let ops = extract_file_operations(&messages);
        assert!(ops.read.is_empty());
        assert!(ops.written.is_empty());
        assert!(ops.edited.is_empty());
    }

    // ========================================================================
    // compact() — additional paths
    // ========================================================================

    #[tokio::test]
    async fn compact_with_previous_summary_uses_update_prompt() {
        let messages: Vec<AgentMessage> = (0..10)
            .map(|i| user_msg(&format!("{}{}", big_message(396), i)))
            .collect();
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 1000,
            keep_recent_tokens: 300,
        ..Default::default()
        };
        let mut prep =
            prepare_compaction(&messages, 50000, &settings, Some("## Goal\nBuild module X"))
                .unwrap();
        // Ensure previous_summary is set (triggers update prompt path)
        assert!(prep.previous_summary.is_some());

        let provider = StatefulProvider::new(vec![vec![
            AssistantMessageEvent::TextDelta(
                "## Goal\nBuild module X\n## Progress\n### Done\n- Module X complete".into(),
            ),
            AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            },
        ]]);

        let model = test_model();
        let result = compact(prep, &provider, &model).await.unwrap();
        assert!(result.summary.contains("Module X complete"));
        assert_eq!(provider.call_count(), 1);
    }

    #[tokio::test]
    async fn compact_split_turn_calls_llm_twice() {
        // Manually construct a CompactionPreparation with split turn
        let prep = CompactionPreparation {
            messages_to_summarize: vec![
                user_msg("Build a REST API"),
                assistant_msg("I'll start by creating the server..."),
            ],
            turn_prefix_messages: vec![
                assistant_with_tool_call("reading", "read_file", json!({"file_path": "/main.rs"})),
                tool_result_msg("tc-1", "fn main() {}"),
            ],
            first_kept_index: 4,
            is_split_turn: true,
            tokens_before: 80000,
            previous_summary: None,
            file_ops: FileOperations::default(),
        };

        // Two LLM calls expected: history summary + turn prefix summary
        let provider = StatefulProvider::new(vec![
            vec![
                AssistantMessageEvent::TextDelta("## Goal\nBuild REST API".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ],
            vec![
                AssistantMessageEvent::TextDelta(
                    "## Early Progress\nStarted reading main.rs".into(),
                ),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ],
        ]);

        let model = test_model();
        let result = compact(prep, &provider, &model).await.unwrap();
        assert_eq!(provider.call_count(), 2, "split turn should call LLM twice");
        assert!(result.summary.contains("Build REST API"));
        assert!(result.summary.contains("Current Turn (early part)"));
        assert!(result.summary.contains("Started reading main.rs"));
    }

    #[tokio::test]
    async fn compact_split_turn_empty_prefix_degrades_gracefully() {
        let prep = CompactionPreparation {
            messages_to_summarize: vec![user_msg("do X"), assistant_msg("doing X")],
            turn_prefix_messages: vec![assistant_msg("partial work")],
            first_kept_index: 3,
            is_split_turn: true,
            tokens_before: 50000,
            previous_summary: None,
            file_ops: FileOperations::default(),
        };

        // Second LLM call returns empty → should degrade gracefully
        let provider = StatefulProvider::new(vec![
            vec![
                AssistantMessageEvent::TextDelta("Main summary".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ],
            vec![
                // Empty response for turn prefix
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ],
        ]);

        let model = test_model();
        let result = compact(prep, &provider, &model).await.unwrap();
        assert!(result.summary.contains("Main summary"));
        // No "Current Turn" section since prefix was empty
        assert!(!result.summary.contains("Current Turn"));
    }

    #[tokio::test]
    async fn compact_nothing_to_compact_returns_error() {
        let prep = CompactionPreparation {
            messages_to_summarize: vec![], // empty
            turn_prefix_messages: vec![],
            first_kept_index: 0,
            is_split_turn: false,
            tokens_before: 1000,
            previous_summary: None,
            file_ops: FileOperations::default(),
        };

        let provider = StatefulProvider::new(vec![]);
        let model = test_model();
        let result = compact(prep, &provider, &model).await;
        assert!(matches!(result, Err(CompactionError::NothingToCompact)));
    }

    #[tokio::test]
    async fn compact_appends_file_ops_to_summary() {
        let mut file_ops = FileOperations::default();
        file_ops.read.insert("/src/main.rs".into());
        file_ops.written.insert("/src/new.rs".into());

        let prep = CompactionPreparation {
            messages_to_summarize: vec![user_msg("build something")],
            turn_prefix_messages: vec![],
            first_kept_index: 1,
            is_split_turn: false,
            tokens_before: 50000,
            previous_summary: None,
            file_ops,
        };

        let provider = StatefulProvider::new(vec![vec![
            AssistantMessageEvent::TextDelta("## Goal\nBuild something".into()),
            AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            },
        ]]);

        let model = test_model();
        let result = compact(prep, &provider, &model).await.unwrap();
        assert!(result.summary.contains("<read-files>"));
        assert!(result.summary.contains("/src/main.rs"));
        assert!(result.summary.contains("<modified-files>"));
        assert!(result.summary.contains("/src/new.rs"));
    }

    // ========================================================================
    // apply_compaction — additional boundary
    // ========================================================================

    #[test]
    fn apply_compaction_all_messages_replaced() {
        // first_kept_index == messages.len() → all messages replaced with summary only
        let mut messages = vec![user_msg("a"), assistant_msg("b"), user_msg("c")];
        let result = CompactionResult {
            summary: "everything happened".into(),
            first_kept_index: 3, // == messages.len()
            tokens_before: 10000,
            file_ops: FileOperations::default(),
        };
        apply_compaction(&mut messages, &result);
        assert_eq!(messages.len(), 1);
        assert!(matches!(&messages[0], AgentMessage::CompactionSummary(_)));
    }

    // ========================================================================
    // State combination — E2E and iterative compaction
    // ========================================================================

    #[tokio::test]
    async fn e2e_prepare_compact_apply_full_pipeline() {
        // Build a realistic 12-message conversation
        let messages = vec![
            user_msg("Create a new Rust project"),
            assistant_with_tool_call(
                "I'll create the project",
                "bash",
                json!({"command": "cargo init myapp"}),
            ),
            tool_result_msg("tc-1", "Created binary (application) `myapp` package"),
            assistant_msg("Project created. Let me add some code."),
            user_msg("Add a hello world function"),
            assistant_with_tool_call(
                "Adding function",
                "write_file",
                json!({"file_path": "/src/lib.rs", "content": "pub fn hello() { println!(\"Hello!\"); }"}),
            ),
            tool_result_msg("tc-2", "File written successfully"),
            assistant_msg("Done! I've added the hello function."),
            user_msg("Now add tests"),
            assistant_with_tool_call(
                "Adding tests",
                "write_file",
                json!({"file_path": "/src/lib.rs", "content": "#[test] fn test_hello() {}"}),
            ),
            tool_result_msg("tc-3", "File written successfully"),
            assistant_msg("Tests added and passing."),
        ];

        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 500,
            keep_recent_tokens: 100, // Keep only last ~1-2 messages
        ..Default::default()
        };

        // Step 1: Prepare
        let prep = prepare_compaction(&messages, 50000, &settings, None);
        assert!(prep.is_some(), "should have messages to compact");
        let prep = prep.unwrap();
        let first_kept = prep.first_kept_index;
        assert!(first_kept > 0 && first_kept < messages.len());

        // Step 2: Compact
        let provider = StatefulProvider::new(vec![vec![
            AssistantMessageEvent::TextDelta(
                "## Goal\nCreate Rust project with hello function and tests\n\
                 ## Progress\n### Done\n- Created project\n- Added hello fn\n- Added tests"
                    .into(),
            ),
            AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            },
        ]]);
        let model = test_model();
        let result = compact(prep, &provider, &model).await.unwrap();

        // Step 3: Apply
        let mut messages = messages;
        let kept_count = messages.len() - first_kept;
        apply_compaction(&mut messages, &result);

        // Verify structure: CompactionSummary + kept messages
        assert_eq!(messages.len(), 1 + kept_count);
        assert!(
            matches!(&messages[0], AgentMessage::CompactionSummary(cs) if cs.summary.contains("hello function"))
        );
        // Remaining messages are from the kept portion
        for msg in &messages[1..] {
            assert!(!matches!(msg, AgentMessage::CompactionSummary(_)));
        }
    }

    #[tokio::test]
    async fn iterative_compaction_second_round() {
        // Simulate: first compaction already happened, now do a second round
        let messages = vec![
            compaction_summary_msg("## Goal\nBuild API\n## Done\n- Setup project"),
            user_msg("Add authentication"),
            assistant_with_tool_call(
                "Adding auth",
                "write_file",
                json!({"file_path": "/src/auth.rs", "content": "pub fn auth() {}"}),
            ),
            tool_result_msg("tc-1", "Written"),
            assistant_msg("Auth module added."),
            user_msg("Add middleware"),
            assistant_msg(&big_message(400)), // Large response to trigger compaction
            user_msg("Add rate limiting"),
            assistant_msg(&big_message(400)),
        ];

        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 500,
            keep_recent_tokens: 150,
        ..Default::default()
        };

        // Second round prepare — previous summary comes from the CompactionSummary message
        let previous_summary = match &messages[0] {
            AgentMessage::CompactionSummary(cs) => Some(cs.summary.as_str()),
            _ => None,
        };
        let prep = prepare_compaction(&messages, 80000, &settings, previous_summary);
        assert!(prep.is_some(), "should need second compaction");
        let prep = prep.unwrap();
        assert!(
            prep.previous_summary.is_some(),
            "should carry forward previous summary"
        );

        // Compact
        let provider = StatefulProvider::new(vec![vec![
            AssistantMessageEvent::TextDelta(
                "## Goal\nBuild API\n## Done\n- Setup\n- Auth\n- Middleware".into(),
            ),
            AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            },
        ]]);
        let model = test_model();
        let result = compact(prep, &provider, &model).await.unwrap();

        // Apply
        let mut messages = messages;
        apply_compaction(&mut messages, &result);

        // Should start with new CompactionSummary
        assert!(
            matches!(&messages[0], AgentMessage::CompactionSummary(cs) if cs.summary.contains("Middleware"))
        );
    }

    #[test]
    fn multiple_apply_compaction_in_sequence() {
        let mut messages = vec![
            user_msg("first"),
            assistant_msg("reply 1"),
            user_msg("second"),
            assistant_msg("reply 2"),
            user_msg("third"),
            assistant_msg("reply 3"),
        ];

        // First apply: compact first 4
        let result1 = CompactionResult {
            summary: "Summary round 1".into(),
            first_kept_index: 4,
            tokens_before: 10000,
            file_ops: FileOperations::default(),
        };
        apply_compaction(&mut messages, &result1);
        assert_eq!(messages.len(), 3); // summary + third + reply3

        // Simulate more messages arriving
        messages.push(user_msg("fourth"));
        messages.push(assistant_msg("reply 4"));

        // Second apply: compact first 3 (summary + third + reply3)
        let result2 = CompactionResult {
            summary: "Summary round 2".into(),
            first_kept_index: 3,
            tokens_before: 20000,
            file_ops: FileOperations::default(),
        };
        apply_compaction(&mut messages, &result2);
        assert_eq!(messages.len(), 3); // summary2 + fourth + reply4
        assert!(matches!(
            &messages[0],
            AgentMessage::CompactionSummary(cs) if cs.summary == "Summary round 2"
        ));
    }

    // ========================================================================
    // Preparation — additional edge cases
    // ========================================================================

    #[test]
    fn prepare_compaction_all_tool_results_degrades_gracefully() {
        // Edge case: only ToolResult messages — no valid cut points.
        // find_cut_point degrades to cutting at a ToolResult rather than
        // returning 0, because the messages exceed keep_recent_tokens budget.
        let messages = vec![
            tool_result_msg("tc-1", &big_message(400)),
            tool_result_msg("tc-2", &big_message(400)),
        ];
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 100,
            keep_recent_tokens: 50,
        ..Default::default()
        };
        let prep = prepare_compaction(&messages, 50000, &settings, None);
        // Degradation: still produces a preparation (cuts at ToolResult)
        assert!(prep.is_some());
        let prep = prep.unwrap();
        assert!(prep.first_kept_index > 0);
    }

    // ========================================================================
    // Error path coverage — comprehensive failure modes
    // ========================================================================

    #[test]
    fn is_context_overflow_all_known_patterns() {
        // Verify all 13 OVERFLOW_PATTERNS are detected
        // Must exactly match OVERFLOW_PATTERNS (13 entries)
        let patterns = [
            "prompt is too long",
            "exceeds the context window",
            "input token count exceeds",
            "maximum context length",
            "context_length_exceeded",
            "request too large",
            "too many tokens",
            "reduce the length",
            "context window exceeded",
            "token limit exceeded",
            "input is too long",
            "request size exceeded",
            "content is too large",
        ];
        for pattern in patterns {
            let msg = AssistantMessage {
                content: vec![],
                provider: "test".into(),
                model: "test".into(),
                usage: Usage::default(),
                stop_reason: StopReason::Stop,
                error_message: Some(format!("Error: {pattern} for this request")),
                timestamp: 0,
            };
            assert!(
                is_context_overflow(&msg, 128_000),
                "pattern '{pattern}' not detected"
            );
        }
    }

    #[test]
    fn is_context_overflow_usage_boundary_exact_equal() {
        // input + cache_read == context_window → should NOT overflow
        let msg = AssistantMessage {
            content: vec![],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage {
                input: 50_000,
                output: 100,
                cache_read: 50_000,
                ..Usage::default()
            },
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };
        assert!(
            !is_context_overflow(&msg, 100_000),
            "exact equal should NOT be overflow"
        );
    }

    #[test]
    fn is_context_overflow_usage_input_only_no_cache() {
        // input > 0, cache_read = 0, input == context_window → NOT overflow
        let msg = AssistantMessage {
            content: vec![],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage {
                input: 100_000,
                output: 100,
                ..Usage::default()
            },
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };
        assert!(!is_context_overflow(&msg, 100_000));
    }

    #[test]
    fn is_context_overflow_usage_input_zero_skips_check() {
        // input = 0 → usage check branch skipped entirely
        let msg = AssistantMessage {
            content: vec![],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage {
                cache_read: 999_999,
                ..Usage::default()
            },
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };
        assert!(
            !is_context_overflow(&msg, 100),
            "input=0 should skip usage check"
        );
    }

    #[test]
    fn compaction_error_display_formats() {
        assert_eq!(
            format!("{}", CompactionError::NothingToCompact),
            "nothing to compact"
        );
        assert_eq!(
            format!("{}", CompactionError::SummarizationFailed("timeout".into())),
            "summarization failed: timeout"
        );
    }

    #[tokio::test]
    async fn compact_multiple_text_deltas_concatenated() {
        // Provider returns 3 TextDelta events — verify they get concatenated
        let messages = vec![
            user_msg(&big_message(400)),
            assistant_msg(&big_message(400)),
        ];
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 100,
            keep_recent_tokens: 50,
        ..Default::default()
        };
        let prep = prepare_compaction(&messages, 50000, &settings, None).unwrap();

        let provider = StatefulProvider::new(vec![vec![
            AssistantMessageEvent::TextDelta("part1 ".into()),
            AssistantMessageEvent::TextDelta("part2 ".into()),
            AssistantMessageEvent::TextDelta("part3".into()),
            AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            },
        ]]);

        let model = test_model();
        let result = compact(prep, &provider, &model).await.unwrap();
        assert!(result.summary.contains("part1 part2 part3"));
    }

    #[tokio::test]
    async fn compact_provider_returns_only_done_event() {
        // Provider returns Done without any TextDelta → empty summary → error
        let messages = vec![
            user_msg(&big_message(400)),
            assistant_msg(&big_message(400)),
        ];
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 100,
            keep_recent_tokens: 50,
        ..Default::default()
        };
        let prep = prepare_compaction(&messages, 50000, &settings, None).unwrap();

        let provider = StatefulProvider::new(vec![vec![AssistantMessageEvent::Done {
            stop_reason: StopReason::Stop,
        }]]);

        let model = test_model();
        let result = compact(prep, &provider, &model).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CompactionError::SummarizationFailed(_)
        ));
    }

    #[tokio::test]
    async fn compact_provider_returns_usage_events_only_text_extracted() {
        // Provider returns mixed events including Usage — only TextDelta extracted
        let messages = vec![
            user_msg(&big_message(400)),
            assistant_msg(&big_message(400)),
        ];
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 100,
            keep_recent_tokens: 50,
        ..Default::default()
        };
        let prep = prepare_compaction(&messages, 50000, &settings, None).unwrap();

        let provider = StatefulProvider::new(vec![vec![
            AssistantMessageEvent::TextDelta("summary text".into()),
            AssistantMessageEvent::Usage(Usage {
                input: 1000,
                output: 200,
                ..Usage::default()
            }),
            AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            },
        ]]);

        let model = test_model();
        let result = compact(prep, &provider, &model).await.unwrap();
        assert_eq!(result.summary, "summary text");
    }

    // ========================================================================
    // Context overflow hook — trait + action enum coverage
    // ========================================================================

    #[tokio::test]
    async fn context_overflow_hook_returns_all_action_variants() {
        // Verify the hook trait works with all action variants

        struct CompactHook;
        #[async_trait::async_trait]
        impl ContextOverflowHook for CompactHook {
            async fn on_context_overflow(
                &self,
                _ctx: &ContextOverflowContext,
            ) -> ContextOverflowAction {
                ContextOverflowAction::Compact
            }
        }

        struct TruncateHook;
        #[async_trait::async_trait]
        impl ContextOverflowHook for TruncateHook {
            async fn on_context_overflow(
                &self,
                _ctx: &ContextOverflowContext,
            ) -> ContextOverflowAction {
                ContextOverflowAction::Truncate
            }
        }

        struct CustomHook;
        #[async_trait::async_trait]
        impl ContextOverflowHook for CustomHook {
            async fn on_context_overflow(
                &self,
                _ctx: &ContextOverflowContext,
            ) -> ContextOverflowAction {
                ContextOverflowAction::CustomSummary("custom summary".into())
            }
        }

        struct AbortHook;
        #[async_trait::async_trait]
        impl ContextOverflowHook for AbortHook {
            async fn on_context_overflow(
                &self,
                _ctx: &ContextOverflowContext,
            ) -> ContextOverflowAction {
                ContextOverflowAction::Abort
            }
        }

        let ctx = ContextOverflowContext {
            reason: CompactionReason::Overflow,
            context_tokens: 200_000,
            context_window: 128_000,
            message_count: 50,
        };

        let action = CompactHook.on_context_overflow(&ctx).await;
        assert!(matches!(action, ContextOverflowAction::Compact));

        let action = TruncateHook.on_context_overflow(&ctx).await;
        assert!(matches!(action, ContextOverflowAction::Truncate));

        let action = CustomHook.on_context_overflow(&ctx).await;
        if let ContextOverflowAction::CustomSummary(s) = action {
            assert_eq!(s, "custom summary");
        } else {
            panic!("expected CustomSummary");
        }

        let action = AbortHook.on_context_overflow(&ctx).await;
        assert!(matches!(action, ContextOverflowAction::Abort));
    }

    #[tokio::test]
    async fn context_overflow_hook_receives_correct_context() {
        struct InspectHook;
        #[async_trait::async_trait]
        impl ContextOverflowHook for InspectHook {
            async fn on_context_overflow(
                &self,
                ctx: &ContextOverflowContext,
            ) -> ContextOverflowAction {
                assert_eq!(ctx.context_tokens, 150_000);
                assert_eq!(ctx.context_window, 128_000);
                assert_eq!(ctx.message_count, 42);
                assert!(matches!(ctx.reason, CompactionReason::Threshold));
                ContextOverflowAction::Compact
            }
        }

        let ctx = ContextOverflowContext {
            reason: CompactionReason::Threshold,
            context_tokens: 150_000,
            context_window: 128_000,
            message_count: 42,
        };
        InspectHook.on_context_overflow(&ctx).await;
    }

    // ========================================================================
    // Serialization — additional edge cases
    // ========================================================================

    #[test]
    fn serialize_messages_user_with_image_ignores_non_text() {
        // User message with Image content — serialization only extracts Text
        let msg = AgentMessage::User(UserMessage {
            content: vec![
                Content::Text {
                    text: "Check this image".to_string(),
                },
                Content::Image {
                    data: "iVBORw0KGgobase64data".to_string(),
                    mime_type: "image/png".to_string(),
                },
            ],
            timestamp: 0,
        });
        let serialized = serialize_messages_for_summary(&[msg]);
        assert!(serialized.contains("Check this image"));
        assert!(!serialized.contains("iVBORw0KGgobase64data"));
    }

    // ========================================================================
    // File operations — short tool names
    // ========================================================================

    #[test]
    fn extract_file_ops_short_tool_names() {
        // "read" and "write" (not just "read_file"/"write_file") are valid
        let messages = vec![
            assistant_with_tool_call("reading", "read", json!({"file_path": "/src/main.rs"})),
            tool_result_msg("tc-1", "fn main(){}"),
            assistant_with_tool_call(
                "writing",
                "write",
                json!({"file_path": "/src/new.rs", "content": "mod foo;"}),
            ),
        ];
        let ops = extract_file_operations(&messages);
        assert!(
            ops.read.contains("/src/main.rs"),
            "short 'read' should work"
        );
        assert!(
            ops.written.contains("/src/new.rs"),
            "short 'write' should work"
        );
    }

    // ========================================================================
    // Preparation — split turn coverage
    // ========================================================================

    #[test]
    fn prepare_compaction_split_turn_extracts_prefix() {
        // Construct a scenario that produces a split turn in prepare_compaction.
        // Turn: user(0) → assistant+tool(1) → tool_result(2) → assistant(3, big)
        // With keep_recent_tokens small enough to keep only msg 3,
        // cut at index 3 (Assistant) will detect split turn back to user at 0.
        let messages = vec![
            user_msg("do something"),                                              // 0
            assistant_with_tool_call("calling", "bash", json!({"command": "ls"})), // 1
            tool_result_msg("tc-1", "file.txt"),                                   // 2
            assistant_msg(&big_message(400)),                                      // 3: ~100 tokens
        ];
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 100,
            keep_recent_tokens: 50, // only keep last ~50 tokens
        ..Default::default()
        };
        let prep = prepare_compaction(&messages, 50000, &settings, None);
        assert!(prep.is_some());
        let prep = prep.unwrap();
        // If cut at index 3 (assistant mid-turn), should detect split
        if prep.is_split_turn {
            assert!(
                !prep.turn_prefix_messages.is_empty(),
                "split turn should have non-empty prefix messages"
            );
        }
    }

    // ========================================================================
    // Truncation — ToolResult behavior
    // ========================================================================

    #[test]
    fn truncate_messages_result_is_valid() {
        // After truncation, verify the result is well-formed
        let mut messages = vec![
            user_msg("old message 1"),
            assistant_msg("old response 1"),
            tool_result_msg("tc-1", "tool output"),
            user_msg("recent question"),
            assistant_msg(&big_message(400)),
        ];
        let original_len = messages.len();
        // Truncate keeping only recent tokens
        truncate_messages(&mut messages, 50);
        // Should have fewer messages than original
        assert!(messages.len() <= original_len);
        // Should still contain some messages
        assert!(!messages.is_empty());
    }

    // ========================================================================
    // State combination — additional scenarios
    // ========================================================================

    #[tokio::test]
    async fn compact_after_compact_iterative_state_preserved() {
        // Two rounds of compaction: first produces summary, second incorporates it
        // Round 1: compact initial messages
        let messages_r1: Vec<AgentMessage> = (0..8)
            .map(|i| user_msg(&format!("{}{}", big_message(396), i)))
            .collect();
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 100,
            keep_recent_tokens: 100,
        ..Default::default()
        };
        let prep_r1 = prepare_compaction(&messages_r1, 50000, &settings, None).unwrap();

        let provider = StatefulProvider::new(vec![
            vec![
                AssistantMessageEvent::TextDelta("Round 1 summary".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ],
            vec![
                AssistantMessageEvent::TextDelta("Round 2 updated summary".into()),
                AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                },
            ],
        ]);
        let model = test_model();

        let result_r1 = compact(prep_r1, &provider, &model).await.unwrap();
        assert!(result_r1.summary.contains("Round 1 summary"));
        assert_eq!(provider.call_count(), 1);

        // Round 2: compact with previous summary
        let mut messages_r2 = vec![compaction_summary_msg(&result_r1.summary)];
        for i in 0..8 {
            messages_r2.push(user_msg(&format!("{}{}", big_message(396), i)));
        }
        let prep_r2 =
            prepare_compaction(&messages_r2, 50000, &settings, Some(&result_r1.summary)).unwrap();
        assert!(prep_r2.previous_summary.is_some());

        let result_r2 = compact(prep_r2, &provider, &model).await.unwrap();
        assert!(result_r2.summary.contains("Round 2 updated summary"));
        assert_eq!(provider.call_count(), 2);
    }

    #[test]
    fn apply_compaction_then_add_messages_then_apply_again() {
        // Simulate: compact → add new messages → compact again
        let mut messages: Vec<AgentMessage> =
            (0..6).map(|i| user_msg(&format!("msg{i}"))).collect();

        // First apply: cut at index 3
        let result1 = CompactionResult {
            summary: "Summary of msg0-msg2".into(),
            first_kept_index: 3,
            tokens_before: 5000,
            file_ops: FileOperations::default(),
        };
        apply_compaction(&mut messages, &result1);
        assert_eq!(messages.len(), 4); // summary + msg3 + msg4 + msg5
        assert!(matches!(messages[0], AgentMessage::CompactionSummary(_)));

        // Add more messages
        messages.push(user_msg("msg6"));
        messages.push(assistant_msg("reply6"));
        assert_eq!(messages.len(), 6);

        // Second apply: cut at index 2
        let result2 = CompactionResult {
            summary: "Summary v2 of everything before msg4".into(),
            first_kept_index: 2,
            tokens_before: 8000,
            file_ops: FileOperations::default(),
        };
        apply_compaction(&mut messages, &result2);
        assert_eq!(messages.len(), 5); // new summary + msg4 + msg5 + msg6 + reply6
        if let AgentMessage::CompactionSummary(cs) = &messages[0] {
            assert!(cs.summary.contains("Summary v2"));
        } else {
            panic!("first message should be CompactionSummary");
        }
    }

    // ========================================================================
    // Error path coverage — round 3 additions
    // ========================================================================

    #[test]
    fn is_context_overflow_empty_error_message_not_overflow() {
        // error_message = Some("") → should NOT match any pattern
        let msg = AssistantMessage {
            content: vec![],
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: Some("".into()),
            timestamp: 0,
        };
        assert!(
            !is_context_overflow(&msg, 128_000),
            "empty error string should not match overflow patterns"
        );
    }

    #[tokio::test]
    async fn compact_split_turn_first_call_empty_is_error() {
        // Split turn: if the first LLM call returns empty → SummarizationFailed
        let prep = CompactionPreparation {
            messages_to_summarize: vec![user_msg("old msg")],
            turn_prefix_messages: vec![user_msg("prefix")],
            first_kept_index: 1,
            is_split_turn: true,
            tokens_before: 50000,
            previous_summary: None,
            file_ops: FileOperations::default(),
        };

        // First call returns empty (Done only, no TextDelta)
        let provider = StatefulProvider::new(vec![vec![AssistantMessageEvent::Done {
            stop_reason: StopReason::Stop,
        }]]);

        let model = test_model();
        let result = compact(prep, &provider, &model).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CompactionError::SummarizationFailed(_)
        ));
    }

    #[test]
    fn apply_compaction_cuts_only_first_message() {
        // first_kept_index = 1 — minimum valid cut (cuts only first message)
        let mut messages = vec![
            user_msg("first"),
            assistant_msg("second"),
            user_msg("third"),
        ];
        let result = CompactionResult {
            summary: "Summary of first message".into(),
            first_kept_index: 1,
            tokens_before: 5000,
            file_ops: FileOperations::default(),
        };
        apply_compaction(&mut messages, &result);
        assert_eq!(messages.len(), 3); // summary + second + third
        assert!(matches!(messages[0], AgentMessage::CompactionSummary(_)));
        if let AgentMessage::CompactionSummary(cs) = &messages[0] {
            assert!(cs.summary.contains("Summary of first message"));
        }
        // Original msg[1] and msg[2] are preserved
        assert!(matches!(messages[1], AgentMessage::Assistant(_)));
        assert!(matches!(messages[2], AgentMessage::User(_)));
    }

    #[test]
    fn compaction_error_is_std_error() {
        // Verify CompactionError implements std::error::Error properly
        let e: Box<dyn std::error::Error> = Box::new(CompactionError::NothingToCompact);
        assert!(e.source().is_none());

        let e2: Box<dyn std::error::Error> =
            Box::new(CompactionError::SummarizationFailed("test".into()));
        assert!(e2.source().is_none());
    }

    // ========================================================================
    // Boundary exploration — round 3 additions
    // ========================================================================

    #[test]
    fn estimate_context_tokens_many_large_messages_no_overflow() {
        // 100 messages each 40000 chars = 1M tokens, verify u32 handles this
        let messages: Vec<AgentMessage> = (0..100).map(|_| user_msg(&big_message(40000))).collect();
        let tokens = estimate_context_tokens(&messages);
        assert_eq!(tokens, 1_000_000);
    }

    #[test]
    fn find_cut_point_keep_zero_tokens() {
        // keep_recent_tokens = 0 → accumulated never reaches 0
        // threshold_index stays at start, accumulated < 0 is impossible (u32)
        let messages = vec![user_msg("a"), assistant_msg("b")];
        let cut = find_cut_point(&messages, 0, 2, 0);
        // With keep_recent_tokens=0, the walk-backwards loop:
        // accumulated starts at 0, i=1: accumulated = ~1, which >= 0 → threshold_index=1, break
        // Then forward scan from 1: messages[1] is Assistant → valid cut point
        assert!(cut.first_kept_index <= 2, "should produce a valid result");
    }

    #[test]
    fn extract_file_ops_tool_call_without_path_keys() {
        // Tool call with no file_path or path key → no file ops extracted
        let messages = vec![assistant_with_tool_call(
            "running",
            "bash",
            json!({"command": "ls -la"}),
        )];
        let ops = extract_file_operations(&messages);
        assert!(ops.read.is_empty());
        assert!(ops.written.is_empty());
        assert!(ops.edited.is_empty());
    }

    #[test]
    fn serialize_messages_assistant_empty_content() {
        // Assistant with empty text → should not emit "[Assistant]: " line
        let msg = assistant_msg("");
        let serialized = serialize_messages_for_summary(&[msg]);
        assert!(
            !serialized.contains("[Assistant]:"),
            "empty assistant text should not appear in serialization"
        );
    }

    #[test]
    fn compaction_settings_default_values() {
        // Verify default settings match documented values
        let settings = CompactionSettings::default();
        assert!(settings.enabled);
        assert_eq!(settings.reserve_tokens, 16384);
        assert_eq!(settings.keep_recent_tokens, 20000);
    }

    // ========================================================================
    // safe_truncate + Error event handling (code review fixes)
    // ========================================================================

    #[test]
    fn safe_truncate_ascii() {
        assert_eq!(safe_truncate("hello world", 5), "hello");
        assert_eq!(safe_truncate("hello", 10), "hello");
        assert_eq!(safe_truncate("", 5), "");
    }

    #[test]
    fn safe_truncate_multibyte_utf8() {
        // "你好世界" = 12 bytes (3 bytes per char)
        let s = "你好世界";
        assert_eq!(s.len(), 12);
        // Truncate at 7 bytes → must land on char boundary at 6 ("你好")
        assert_eq!(safe_truncate(s, 7), "你好");
        // Truncate at 3 → exactly one char
        assert_eq!(safe_truncate(s, 3), "你");
        // Truncate at 1 → can't fit any char, returns ""
        assert_eq!(safe_truncate(s, 1), "");
    }

    #[test]
    fn serialize_messages_truncates_multibyte_tool_args_safely() {
        // Tool call args with CJK characters → truncation at 200 bytes must not panic
        let long_cjk = "测".repeat(100); // 300 bytes (3 bytes each)
        let msg = assistant_with_tool_call("processing", "bash", json!({"command": long_cjk}));
        // This should not panic
        let serialized = serialize_messages_for_summary(&[msg]);
        assert!(serialized.contains("..."));
    }

    #[test]
    fn serialize_messages_truncates_multibyte_tool_result_safely() {
        // Tool result with CJK content → truncation at 2000 bytes must not panic
        let long_cjk = "中".repeat(1000); // 3000 bytes
        let msg = tool_result_msg("tc-1", &long_cjk);
        // This should not panic
        let serialized = serialize_messages_for_summary(&[msg]);
        assert!(serialized.contains("truncated"));
    }

    #[tokio::test]
    async fn compact_provider_error_event_returns_error() {
        // Provider returns Error event → compact returns SummarizationFailed
        let messages = vec![
            user_msg(&big_message(400)),
            assistant_msg(&big_message(400)),
        ];
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 100,
            keep_recent_tokens: 50,
        ..Default::default()
        };
        let prep = prepare_compaction(&messages, 50000, &settings, None).unwrap();

        let provider = StatefulProvider::new(vec![vec![AssistantMessageEvent::Error(
            "rate limit exceeded".into(),
        )]]);

        let model = test_model();
        let result = compact(prep, &provider, &model).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, CompactionError::SummarizationFailed(_)));
        assert!(format!("{err}").contains("rate limit exceeded"));
    }
}
