/// Context compaction for long sessions.
///
/// Mirrors pi-mono packages/coding-agent/src/core/compaction/compaction.ts
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::compaction::utils::{
    FileOperations, SUMMARIZATION_SYSTEM_PROMPT, compute_file_lists, create_file_ops,
    extract_file_ops_from_message, format_file_operations, serialize_conversation,
};
use crate::core::session_manager::{CompactionEntry, SessionEntry};

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionDetails {
    #[serde(rename = "readFiles")]
    pub read_files: Vec<String>,
    #[serde(rename = "modifiedFiles")]
    pub modified_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResult {
    pub summary: String,
    #[serde(rename = "firstKeptEntryId")]
    pub first_kept_entry_id: String,
    #[serde(rename = "tokensBefore")]
    pub tokens_before: u64,
    pub details: Option<CompactionDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionSettings {
    pub enabled: bool,
    #[serde(rename = "reserveTokens")]
    pub reserve_tokens: u64,
    #[serde(rename = "keepRecentTokens")]
    pub keep_recent_tokens: u64,
}

impl Default for CompactionSettings {
    fn default() -> Self {
        DEFAULT_COMPACTION_SETTINGS
    }
}

pub const DEFAULT_COMPACTION_SETTINGS: CompactionSettings = CompactionSettings {
    enabled: true,
    reserve_tokens: 16384,
    keep_recent_tokens: 20000,
};

// ============================================================================
// Token calculation
// ============================================================================

/// Calculate total context tokens from a usage Value.
pub fn calculate_context_tokens(usage: &Value) -> u64 {
    if let Some(total) = usage.get("totalTokens").and_then(|t| t.as_u64()) {
        if total > 0 {
            return total;
        }
    }
    let input = usage.get("input").and_then(|v| v.as_u64()).unwrap_or(0);
    let output = usage.get("output").and_then(|v| v.as_u64()).unwrap_or(0);
    let cache_read = usage.get("cacheRead").and_then(|v| v.as_u64()).unwrap_or(0);
    let cache_write = usage
        .get("cacheWrite")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    input + output + cache_read + cache_write
}

/// Get usage from an assistant message if it's non-aborted.
fn get_assistant_usage(msg: &Value) -> Option<&Value> {
    let role = msg.get("role").and_then(|r| r.as_str())?;
    if role != "assistant" {
        return None;
    }
    let stop_reason = msg.get("stopReason").and_then(|s| s.as_str()).unwrap_or("");
    if stop_reason == "aborted" || stop_reason == "error" {
        return None;
    }
    msg.get("usage")
}

/// Find the last non-aborted assistant message usage from session entries.
pub fn get_last_assistant_usage(entries: &[SessionEntry]) -> Option<Value> {
    for entry in entries.iter().rev() {
        if let SessionEntry::Message(e) = entry {
            if let Some(usage) = get_assistant_usage(&e.message) {
                return Some(usage.clone());
            }
        }
    }
    None
}

#[derive(Debug, Clone)]
pub struct ContextUsageEstimate {
    pub tokens: u64,
    pub usage_tokens: u64,
    pub trailing_tokens: u64,
    pub last_usage_index: Option<usize>,
}

fn get_last_assistant_usage_info(messages: &[Value]) -> Option<(Value, usize)> {
    for (i, msg) in messages.iter().enumerate().rev() {
        if let Some(usage) = get_assistant_usage(msg) {
            return Some((usage.clone(), i));
        }
    }
    None
}

/// Estimate context tokens from a list of messages.
pub fn estimate_context_tokens(messages: &[Value]) -> ContextUsageEstimate {
    let usage_info = get_last_assistant_usage_info(messages);

    if let Some((usage, idx)) = usage_info {
        let usage_tokens = calculate_context_tokens(&usage);
        let trailing_tokens: u64 = messages[idx + 1..].iter().map(|m| estimate_tokens(m)).sum();
        ContextUsageEstimate {
            tokens: usage_tokens + trailing_tokens,
            usage_tokens,
            trailing_tokens,
            last_usage_index: Some(idx),
        }
    } else {
        let estimated: u64 = messages.iter().map(|m| estimate_tokens(m)).sum();
        ContextUsageEstimate {
            tokens: estimated,
            usage_tokens: 0,
            trailing_tokens: estimated,
            last_usage_index: None,
        }
    }
}

/// Check if compaction should trigger.
pub fn should_compact(
    context_tokens: u64,
    context_window: u64,
    settings: &CompactionSettings,
) -> bool {
    if !settings.enabled {
        return false;
    }
    context_tokens > context_window - settings.reserve_tokens
}

// ============================================================================
// Token estimation
// ============================================================================

/// Estimate token count for a message (JSON Value) using chars/4 heuristic.
pub fn estimate_tokens(message: &Value) -> u64 {
    let role = message.get("role").and_then(|r| r.as_str()).unwrap_or("");
    let chars: usize = match role {
        "user" => {
            let content = message.get("content");
            match content {
                Some(Value::String(s)) => s.len(),
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|block| {
                        if block.get("type")?.as_str()? == "text" {
                            block.get("text")?.as_str().map(|t| t.len())
                        } else {
                            None
                        }
                    })
                    .sum(),
                _ => 0,
            }
        }
        "assistant" => {
            let content = match message.get("content").and_then(|c| c.as_array()) {
                Some(arr) => arr,
                None => return 0,
            };
            let mut chars = 0;
            for block in content {
                let t = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match t {
                    "text" => {
                        chars += block
                            .get("text")
                            .and_then(|t| t.as_str())
                            .map(|s| s.len())
                            .unwrap_or(0);
                    }
                    "thinking" => {
                        chars += block
                            .get("thinking")
                            .and_then(|t| t.as_str())
                            .map(|s| s.len())
                            .unwrap_or(0);
                    }
                    "toolCall" => {
                        chars += block
                            .get("name")
                            .and_then(|n| n.as_str())
                            .map(|s| s.len())
                            .unwrap_or(0);
                        if let Some(args) = block.get("arguments") {
                            chars += args.to_string().len();
                        }
                    }
                    _ => {}
                }
            }
            chars
        }
        "custom" | "toolResult" => {
            let content = message.get("content");
            match content {
                Some(Value::String(s)) => s.len(),
                Some(Value::Array(arr)) => {
                    let mut chars = 0;
                    for block in arr {
                        let t = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if t == "text" {
                            chars += block
                                .get("text")
                                .and_then(|t| t.as_str())
                                .map(|s| s.len())
                                .unwrap_or(0);
                        }
                        if t == "image" {
                            chars += 4800; // ~1200 tokens
                        }
                    }
                    chars
                }
                _ => 0,
            }
        }
        "bashExecution" => {
            let cmd = message
                .get("command")
                .and_then(|c| c.as_str())
                .map(|s| s.len())
                .unwrap_or(0);
            let out = message
                .get("output")
                .and_then(|o| o.as_str())
                .map(|s| s.len())
                .unwrap_or(0);
            cmd + out
        }
        "branchSummary" | "compactionSummary" => message
            .get("summary")
            .and_then(|s| s.as_str())
            .map(|s| s.len())
            .unwrap_or(0),
        _ => 0,
    };
    (chars as u64 + 3) / 4
}

// ============================================================================
// Cut point detection
// ============================================================================

fn find_valid_cut_points(
    entries: &[SessionEntry],
    start_index: usize,
    end_index: usize,
) -> Vec<usize> {
    let mut cut_points = Vec::new();
    for i in start_index..end_index {
        let entry = &entries[i];
        match entry {
            SessionEntry::Message(e) => {
                let role = e.message.get("role").and_then(|r| r.as_str()).unwrap_or("");
                match role {
                    "bashExecution" | "custom" | "branchSummary" | "compactionSummary" | "user"
                    | "assistant" => {
                        cut_points.push(i);
                    }
                    "toolResult" => {}
                    _ => {}
                }
            }
            SessionEntry::BranchSummary(_) | SessionEntry::CustomMessage(_) => {
                cut_points.push(i);
            }
            _ => {}
        }
    }
    cut_points
}

/// Find the user message (or bashExecution) that starts the turn containing the given entry.
pub fn find_turn_start_index(
    entries: &[SessionEntry],
    entry_index: usize,
    start_index: usize,
) -> Option<usize> {
    let mut i = entry_index as isize;
    while i >= start_index as isize {
        let entry = &entries[i as usize];
        match entry {
            SessionEntry::BranchSummary(_) | SessionEntry::CustomMessage(_) => {
                return Some(i as usize);
            }
            SessionEntry::Message(e) => {
                let role = e.message.get("role").and_then(|r| r.as_str()).unwrap_or("");
                if role == "user" || role == "bashExecution" {
                    return Some(i as usize);
                }
            }
            _ => {}
        }
        i -= 1;
    }
    None
}

#[derive(Debug, Clone)]
pub struct CutPointResult {
    pub first_kept_entry_index: usize,
    pub turn_start_index: Option<usize>,
    pub is_split_turn: bool,
}

/// Find the cut point that keeps approximately `keep_recent_tokens`.
pub fn find_cut_point(
    entries: &[SessionEntry],
    start_index: usize,
    end_index: usize,
    keep_recent_tokens: u64,
) -> CutPointResult {
    let cut_points = find_valid_cut_points(entries, start_index, end_index);

    if cut_points.is_empty() {
        return CutPointResult {
            first_kept_entry_index: start_index,
            turn_start_index: None,
            is_split_turn: false,
        };
    }

    let mut accumulated_tokens = 0u64;
    let mut cut_index = cut_points[0];

    let mut i = end_index as isize - 1;
    while i >= start_index as isize {
        let entry = &entries[i as usize];
        if let SessionEntry::Message(e) = entry {
            let message_tokens = estimate_tokens(&e.message);
            accumulated_tokens += message_tokens;

            if accumulated_tokens >= keep_recent_tokens {
                // Find the closest valid cut point at or after this entry
                for &cp in &cut_points {
                    if cp >= i as usize {
                        cut_index = cp;
                        break;
                    }
                }
                break;
            }
        }
        i -= 1;
    }

    // Scan backwards from cut_index to include non-message entries
    while cut_index > start_index {
        let prev_entry = &entries[cut_index - 1];
        match prev_entry {
            SessionEntry::Compaction(_) => break,
            SessionEntry::Message(_) => break,
            _ => {
                cut_index -= 1;
            }
        }
    }

    // Determine if this is a split turn
    let is_user_message = match &entries[cut_index] {
        SessionEntry::Message(e) => e.message.get("role").and_then(|r| r.as_str()) == Some("user"),
        _ => false,
    };

    let turn_start_index = if is_user_message {
        None
    } else {
        find_turn_start_index(entries, cut_index, start_index)
    };

    CutPointResult {
        first_kept_entry_index: cut_index,
        turn_start_index,
        is_split_turn: !is_user_message && turn_start_index.is_some(),
    }
}

// ============================================================================
// Message extraction from entries
// ============================================================================

fn get_message_from_entry(entry: &SessionEntry) -> Option<Value> {
    match entry {
        SessionEntry::Message(e) => Some(e.message.clone()),
        SessionEntry::CustomMessage(e) => Some(serde_json::json!({
            "role": "custom",
            "customType": e.custom_type,
            "content": e.content,
            "display": e.display,
            "timestamp": e.timestamp
        })),
        SessionEntry::BranchSummary(e) => Some(serde_json::json!({
            "role": "branchSummary",
            "summary": e.summary,
            "fromId": e.from_id,
            "timestamp": e.timestamp
        })),
        SessionEntry::Compaction(e) => Some(serde_json::json!({
            "role": "compactionSummary",
            "summary": e.summary,
            "tokensBefore": e.tokens_before,
            "timestamp": e.timestamp
        })),
        _ => None,
    }
}

// ============================================================================
// Compaction preparation
// ============================================================================

pub struct CompactionPreparation {
    pub first_kept_entry_id: String,
    pub messages_to_summarize: Vec<Value>,
    pub turn_prefix_messages: Vec<Value>,
    pub is_split_turn: bool,
    pub tokens_before: u64,
    pub previous_summary: Option<String>,
    pub file_ops: FileOperations,
    pub settings: CompactionSettings,
}

pub fn prepare_compaction(
    path_entries: &[SessionEntry],
    settings: &CompactionSettings,
) -> Option<CompactionPreparation> {
    // Don't compact if last entry is already a compaction
    if path_entries
        .last()
        .map(|e| matches!(e, SessionEntry::Compaction(_)))
        .unwrap_or(false)
    {
        return None;
    }

    // Find previous compaction
    let prev_compaction_index = path_entries
        .iter()
        .enumerate()
        .rev()
        .find(|(_, e)| matches!(e, SessionEntry::Compaction(_)))
        .map(|(i, _)| i);

    let boundary_start = prev_compaction_index.map(|i| i + 1).unwrap_or(0);
    let boundary_end = path_entries.len();

    // Estimate tokens
    let usage_start = prev_compaction_index.unwrap_or(0);
    let usage_messages: Vec<Value> = (usage_start..boundary_end)
        .filter_map(|i| get_message_from_entry(&path_entries[i]))
        .collect();
    let tokens_before = estimate_context_tokens(&usage_messages).tokens;

    let cut_point = find_cut_point(
        path_entries,
        boundary_start,
        boundary_end,
        settings.keep_recent_tokens,
    );

    // Get ID of first kept entry
    let first_kept_entry = &path_entries[cut_point.first_kept_entry_index];
    let first_kept_entry_id = first_kept_entry.id().to_string();
    if first_kept_entry_id.is_empty() {
        return None; // needs migration
    }

    let history_end = if cut_point.is_split_turn {
        cut_point
            .turn_start_index
            .unwrap_or(cut_point.first_kept_entry_index)
    } else {
        cut_point.first_kept_entry_index
    };

    // Messages to summarize
    let messages_to_summarize: Vec<Value> = (boundary_start..history_end)
        .filter_map(|i| get_message_from_entry(&path_entries[i]))
        .collect();

    // Turn prefix messages (if splitting)
    let turn_prefix_messages: Vec<Value> = if cut_point.is_split_turn {
        let turn_start = cut_point
            .turn_start_index
            .unwrap_or(cut_point.first_kept_entry_index);
        (turn_start..cut_point.first_kept_entry_index)
            .filter_map(|i| get_message_from_entry(&path_entries[i]))
            .collect()
    } else {
        Vec::new()
    };

    // Previous summary for iterative update
    let previous_summary = prev_compaction_index.and_then(|i| {
        if let SessionEntry::Compaction(c) = &path_entries[i] {
            Some(c.summary.clone())
        } else {
            None
        }
    });

    // Extract file operations
    let mut file_ops =
        extract_file_operations(&messages_to_summarize, path_entries, prev_compaction_index);

    if cut_point.is_split_turn {
        for msg in &turn_prefix_messages {
            extract_file_ops_from_message(msg, &mut file_ops);
        }
    }

    Some(CompactionPreparation {
        first_kept_entry_id,
        messages_to_summarize,
        turn_prefix_messages,
        is_split_turn: cut_point.is_split_turn,
        tokens_before,
        previous_summary,
        file_ops,
        settings: settings.clone(),
    })
}

fn extract_file_operations(
    messages: &[Value],
    entries: &[SessionEntry],
    prev_compaction_index: Option<usize>,
) -> FileOperations {
    let mut file_ops = create_file_ops();

    // Collect from previous compaction's details (if pi-generated)
    if let Some(idx) = prev_compaction_index {
        if let SessionEntry::Compaction(prev) = &entries[idx] {
            if prev.from_hook != Some(true) {
                if let Some(details) = &prev.details {
                    if let Some(read_files) = details.get("readFiles").and_then(|r| r.as_array()) {
                        for f in read_files {
                            if let Some(s) = f.as_str() {
                                file_ops.read.insert(s.to_string());
                            }
                        }
                    }
                    if let Some(modified_files) =
                        details.get("modifiedFiles").and_then(|m| m.as_array())
                    {
                        for f in modified_files {
                            if let Some(s) = f.as_str() {
                                file_ops.edited.insert(s.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // Extract from tool calls
    for msg in messages {
        extract_file_ops_from_message(msg, &mut file_ops);
    }

    file_ops
}

// ============================================================================
// Summarization prompts
// ============================================================================

const SUMMARIZATION_PROMPT: &str = "The messages above are a conversation to summarize. \
Create a structured context checkpoint summary that another LLM will use to continue the work.\n\n\
Use this EXACT format:\n\n\
## Goal\n\
[What is the user trying to accomplish? Can be multiple items if the session covers different tasks.]\n\n\
## Constraints & Preferences\n\
- [Any constraints, preferences, or requirements mentioned by user]\n\
- [Or \"(none)\" if none were mentioned]\n\n\
## Progress\n\
### Done\n\
- [x] [Completed tasks/changes]\n\n\
### In Progress\n\
- [ ] [Current work]\n\n\
### Blocked\n\
- [Issues preventing progress, if any]\n\n\
## Key Decisions\n\
- **[Decision]**: [Brief rationale]\n\n\
## Next Steps\n\
1. [Ordered list of what should happen next]\n\n\
## Critical Context\n\
- [Any data, examples, or references needed to continue]\n\
- [Or \"(none)\" if not applicable]\n\n\
Keep each section concise. Preserve exact file paths, function names, and error messages.";

const UPDATE_SUMMARIZATION_PROMPT: &str = "The messages above are NEW conversation messages \
to incorporate into the existing summary provided in <previous-summary> tags.\n\n\
Update the existing structured summary with new information. RULES:\n\
- PRESERVE all existing information from the previous summary\n\
- ADD new progress, decisions, and context from the new messages\n\
- UPDATE the Progress section: move items from \"In Progress\" to \"Done\" when completed\n\
- UPDATE \"Next Steps\" based on what was accomplished\n\
- PRESERVE exact file paths, function names, and error messages\n\
- If something is no longer relevant, you may remove it\n\n\
Use this EXACT format:\n\n\
## Goal\n\
[Preserve existing goals, add new ones if the task expanded]\n\n\
## Constraints & Preferences\n\
- [Preserve existing, add new ones discovered]\n\n\
## Progress\n\
### Done\n\
- [x] [Include previously done items AND newly completed items]\n\n\
### In Progress\n\
- [ ] [Current work - update based on progress]\n\n\
### Blocked\n\
- [Current blockers - remove if resolved]\n\n\
## Key Decisions\n\
- **[Decision]**: [Brief rationale] (preserve all previous, add new)\n\n\
## Next Steps\n\
1. [Update based on current state]\n\n\
## Critical Context\n\
- [Preserve important context, add new if needed]\n\n\
Keep each section concise. Preserve exact file paths, function names, and error messages.";

const TURN_PREFIX_SUMMARIZATION_PROMPT: &str = "This is the PREFIX of a turn that was too large \
to keep. The SUFFIX (recent work) is retained.\n\n\
Summarize the prefix to provide context for the retained suffix:\n\n\
## Original Request\n\
[What did the user ask for in this turn?]\n\n\
## Early Progress\n\
- [Key decisions and work done in the prefix]\n\n\
## Context for Suffix\n\
- [Information needed to understand the retained recent work]\n\n\
Be concise. Focus on what's needed to understand the kept suffix.";

// ============================================================================
// LLM summarization (async — requires AI client)
// ============================================================================

/// Generate a summary using a generic LLM client function.
/// The caller provides a closure that takes (system_prompt, messages_json, max_tokens)
/// and returns the summary string.
pub async fn generate_summary<F, Fut>(
    current_messages: &[Value],
    reserve_tokens: u64,
    custom_instructions: Option<&str>,
    previous_summary: Option<&str>,
    call_llm: F,
) -> anyhow::Result<String>
where
    F: Fn(String, Vec<Value>, u64) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<String>>,
{
    let max_tokens = (reserve_tokens as f64 * 0.8) as u64;

    let mut base_prompt = if previous_summary.is_some() {
        UPDATE_SUMMARIZATION_PROMPT.to_string()
    } else {
        SUMMARIZATION_PROMPT.to_string()
    };

    if let Some(instructions) = custom_instructions {
        base_prompt = format!("{}\n\nAdditional focus: {}", base_prompt, instructions);
    }

    let conversation_text = serialize_conversation(current_messages);
    let mut prompt_text = format!("<conversation>\n{}\n</conversation>\n\n", conversation_text);

    if let Some(prev) = previous_summary {
        prompt_text.push_str(&format!(
            "<previous-summary>\n{}\n</previous-summary>\n\n",
            prev
        ));
    }
    prompt_text.push_str(&base_prompt);

    let summarization_messages = vec![serde_json::json!({
        "role": "user",
        "content": [{"type": "text", "text": prompt_text}],
        "timestamp": chrono::Utc::now().timestamp_millis()
    })];

    call_llm(
        SUMMARIZATION_SYSTEM_PROMPT.to_string(),
        summarization_messages,
        max_tokens,
    )
    .await
}

/// Run the full compaction pipeline using a LLM caller closure.
pub async fn compact<F, Fut>(
    preparation: &CompactionPreparation,
    custom_instructions: Option<&str>,
    call_llm: F,
) -> anyhow::Result<CompactionResult>
where
    F: Fn(String, Vec<Value>, u64) -> Fut + Clone,
    Fut: std::future::Future<Output = anyhow::Result<String>>,
{
    let summary = if preparation.is_split_turn && !preparation.turn_prefix_messages.is_empty() {
        let history_future = if !preparation.messages_to_summarize.is_empty() {
            let cloned_call_llm = call_llm.clone();
            let msgs = preparation.messages_to_summarize.clone();
            let prev = preparation.previous_summary.clone();
            let instructions = custom_instructions.map(|s| s.to_string());
            let reserve = preparation.settings.reserve_tokens;
            Some(async move {
                generate_summary(
                    &msgs,
                    reserve,
                    instructions.as_deref(),
                    prev.as_deref(),
                    cloned_call_llm,
                )
                .await
            })
        } else {
            None
        };

        let turn_prefix_future = {
            let cloned_call_llm = call_llm.clone();
            let msgs = preparation.turn_prefix_messages.clone();
            let reserve = preparation.settings.reserve_tokens;
            async move { generate_turn_prefix_summary(&msgs, reserve, cloned_call_llm).await }
        };

        if let Some(history_fut) = history_future {
            let (history_result, turn_prefix_result) =
                tokio::join!(history_fut, turn_prefix_future);
            let history_summary =
                history_result.unwrap_or_else(|_| "No prior history.".to_string());
            let turn_prefix_summary = turn_prefix_result?;
            format!(
                "{}\n\n---\n\n**Turn Context (split turn):**\n\n{}",
                history_summary, turn_prefix_summary
            )
        } else {
            let turn_prefix_summary = turn_prefix_future.await?;
            format!(
                "No prior history.\n\n---\n\n**Turn Context (split turn):**\n\n{}",
                turn_prefix_summary
            )
        }
    } else {
        generate_summary(
            &preparation.messages_to_summarize,
            preparation.settings.reserve_tokens,
            custom_instructions,
            preparation.previous_summary.as_deref(),
            call_llm,
        )
        .await?
    };

    let (read_files, modified_files) = compute_file_lists(&preparation.file_ops);
    let full_summary = format!(
        "{}{}",
        summary,
        format_file_operations(&read_files, &modified_files)
    );

    if preparation.first_kept_entry_id.is_empty() {
        anyhow::bail!("First kept entry has no ID - session may need migration");
    }

    Ok(CompactionResult {
        summary: full_summary,
        first_kept_entry_id: preparation.first_kept_entry_id.clone(),
        tokens_before: preparation.tokens_before,
        details: Some(CompactionDetails {
            read_files,
            modified_files,
        }),
    })
}

async fn generate_turn_prefix_summary<F, Fut>(
    messages: &[Value],
    reserve_tokens: u64,
    call_llm: F,
) -> anyhow::Result<String>
where
    F: Fn(String, Vec<Value>, u64) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<String>>,
{
    let max_tokens = (reserve_tokens as f64 * 0.5) as u64;
    let conversation_text = serialize_conversation(messages);
    let prompt_text = format!(
        "<conversation>\n{}\n</conversation>\n\n{}",
        conversation_text, TURN_PREFIX_SUMMARIZATION_PROMPT
    );

    let summarization_messages = vec![serde_json::json!({
        "role": "user",
        "content": [{"type": "text", "text": prompt_text}],
        "timestamp": chrono::Utc::now().timestamp_millis()
    })];

    call_llm(
        SUMMARIZATION_SYSTEM_PROMPT.to_string(),
        summarization_messages,
        max_tokens,
    )
    .await
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::session_manager::{
        CompactionEntry, ModelChangeEntry, SessionEntry, SessionMessageEntry,
        ThinkingLevelChangeEntry,
    };

    fn make_usage(input: u64, output: u64, cache_read: u64, cache_write: u64) -> Value {
        serde_json::json!({
            "input": input,
            "output": output,
            "cacheRead": cache_read,
            "cacheWrite": cache_write,
            "totalTokens": input + output + cache_read + cache_write,
            "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}
        })
    }

    fn make_user_msg(text: &str) -> Value {
        serde_json::json!({
            "role": "user",
            "content": text,
            "timestamp": 1
        })
    }

    fn make_assistant_msg(text: &str, usage: Value) -> Value {
        serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": text}],
            "usage": usage,
            "stopReason": "stop",
            "timestamp": 1,
            "api": "anthropic-messages",
            "provider": "anthropic",
            "model": "claude-sonnet-4-5"
        })
    }

    fn make_msg_entry(id: &str, parent_id: Option<&str>, message: Value) -> SessionEntry {
        SessionEntry::Message(SessionMessageEntry {
            entry_type: "message".to_string(),
            id: id.to_string(),
            parent_id: parent_id.map(|s| s.to_string()),
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            message,
        })
    }

    fn make_compaction_entry_se(
        id: &str,
        parent_id: Option<&str>,
        summary: &str,
        first_kept_entry_id: &str,
    ) -> SessionEntry {
        SessionEntry::Compaction(CompactionEntry {
            entry_type: "compaction".to_string(),
            id: id.to_string(),
            parent_id: parent_id.map(|s| s.to_string()),
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            summary: summary.to_string(),
            first_kept_entry_id: first_kept_entry_id.to_string(),
            tokens_before: 10000,
            details: None,
            from_hook: None,
        })
    }

    // --- calculate_context_tokens ---

    #[test]
    fn test_calculate_context_tokens() {
        let usage = make_usage(1000, 500, 200, 100);
        assert_eq!(calculate_context_tokens(&usage), 1800);
    }

    #[test]
    fn test_calculate_context_tokens_zero() {
        let usage = make_usage(0, 0, 0, 0);
        assert_eq!(calculate_context_tokens(&usage), 0);
    }

    // --- get_last_assistant_usage ---

    #[test]
    fn test_get_last_assistant_usage() {
        let entries = vec![
            make_msg_entry("0", None, make_user_msg("Hello")),
            make_msg_entry(
                "1",
                Some("0"),
                make_assistant_msg("Hi", make_usage(100, 50, 0, 0)),
            ),
            make_msg_entry("2", Some("1"), make_user_msg("How are you?")),
            make_msg_entry(
                "3",
                Some("2"),
                make_assistant_msg("Good", make_usage(200, 100, 0, 0)),
            ),
        ];

        let usage = get_last_assistant_usage(&entries).unwrap();
        assert_eq!(usage.get("input").and_then(|v| v.as_u64()), Some(200));
    }

    #[test]
    fn test_get_last_assistant_usage_skips_aborted() {
        let mut aborted = make_assistant_msg("Aborted", make_usage(300, 150, 0, 0));
        aborted.as_object_mut().unwrap().insert(
            "stopReason".to_string(),
            Value::String("aborted".to_string()),
        );

        let entries = vec![
            make_msg_entry("0", None, make_user_msg("Hello")),
            make_msg_entry(
                "1",
                Some("0"),
                make_assistant_msg("Hi", make_usage(100, 50, 0, 0)),
            ),
            make_msg_entry("2", Some("1"), make_user_msg("How are you?")),
            make_msg_entry("3", Some("2"), aborted),
        ];

        let usage = get_last_assistant_usage(&entries).unwrap();
        assert_eq!(usage.get("input").and_then(|v| v.as_u64()), Some(100));
    }

    #[test]
    fn test_get_last_assistant_usage_none() {
        let entries = vec![make_msg_entry("0", None, make_user_msg("Hello"))];
        assert!(get_last_assistant_usage(&entries).is_none());
    }

    // --- should_compact ---

    #[test]
    fn test_should_compact_true() {
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 10000,
            keep_recent_tokens: 20000,
        };
        assert!(should_compact(95000, 100000, &settings));
        assert!(!should_compact(89000, 100000, &settings));
    }

    #[test]
    fn test_should_compact_disabled() {
        let settings = CompactionSettings {
            enabled: false,
            reserve_tokens: 10000,
            keep_recent_tokens: 20000,
        };
        assert!(!should_compact(95000, 100000, &settings));
    }

    // --- find_cut_point ---

    #[test]
    fn test_find_cut_point_returns_start_if_no_cut_points() {
        let entries = vec![make_msg_entry(
            "0",
            None,
            make_assistant_msg("a", make_usage(100, 50, 0, 0)),
        )];
        let result = find_cut_point(&entries, 0, entries.len(), 1000);
        assert_eq!(result.first_kept_entry_index, 0);
    }

    #[test]
    fn test_find_cut_point_keeps_everything_when_within_budget() {
        let entries = vec![
            make_msg_entry("0", None, make_user_msg("1")),
            make_msg_entry(
                "1",
                Some("0"),
                make_assistant_msg("a", make_usage(0, 50, 500, 0)),
            ),
            make_msg_entry("2", Some("1"), make_user_msg("2")),
            make_msg_entry(
                "3",
                Some("2"),
                make_assistant_msg("b", make_usage(0, 50, 1000, 0)),
            ),
        ];
        let result = find_cut_point(&entries, 0, entries.len(), 50000);
        assert_eq!(result.first_kept_entry_index, 0);
    }

    #[test]
    fn test_find_cut_point_valid_entry_type() {
        let u_ids: Vec<String> = (0..10).map(|i| format!("u{}", i)).collect();
        let a_ids: Vec<String> = (0..10).map(|i| format!("a{}", i)).collect();
        let u_texts: Vec<String> = (0..10).map(|i| format!("User {}", i)).collect();
        let a_texts: Vec<String> = (0..10).map(|i| format!("Asst {}", i)).collect();

        let mut entries = vec![];
        for i in 0..10 {
            entries.push(make_msg_entry(
                &u_ids[i],
                if i == 0 { None } else { Some(&a_ids[i - 1]) },
                make_user_msg(&u_texts[i]),
            ));
            entries.push(make_msg_entry(
                &a_ids[i],
                Some(&u_ids[i]),
                make_assistant_msg(&a_texts[i], make_usage(0, 100, (i as u64 + 1) * 1000, 0)),
            ));
        }

        let result = find_cut_point(&entries, 0, entries.len(), 2500);
        let cut_entry = &entries[result.first_kept_entry_index];
        assert!(matches!(cut_entry, SessionEntry::Message(_)));
    }
}
