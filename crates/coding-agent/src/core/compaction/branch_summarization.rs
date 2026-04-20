/// Branch summarization for tree navigation.
///
/// Mirrors pi-mono packages/coding-agent/src/core/compaction/branch-summarization.ts
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::compaction::compaction::estimate_tokens;
use crate::core::compaction::utils::{
    compute_file_lists, create_file_ops, extract_file_ops_from_message, format_file_operations,
    serialize_conversation, FileOperations, SUMMARIZATION_SYSTEM_PROMPT,
};
use crate::core::session_manager::SessionEntry;

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchSummaryResult {
    pub summary: Option<String>,
    pub read_files: Option<Vec<String>>,
    pub modified_files: Option<Vec<String>>,
    pub aborted: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchSummaryDetails {
    #[serde(rename = "readFiles")]
    pub read_files: Vec<String>,
    #[serde(rename = "modifiedFiles")]
    pub modified_files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BranchPreparation {
    pub messages: Vec<Value>,
    pub file_ops: FileOperations,
    pub total_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct CollectEntriesResult {
    pub entries: Vec<SessionEntry>,
    pub common_ancestor_id: Option<String>,
}

// ============================================================================
// Entry Collection
// ============================================================================

/// A minimal read-only view of the session for branch summarization.
/// The caller provides closures for `get_branch` and `get_entry`.
pub fn collect_entries_for_branch_summary<GetBranch, GetEntry>(
    old_leaf_id: Option<&str>,
    target_id: &str,
    get_branch: GetBranch,
    get_entry: GetEntry,
) -> CollectEntriesResult
where
    GetBranch: Fn(&str) -> Vec<SessionEntry>,
    GetEntry: Fn(&str) -> Option<SessionEntry>,
{
    let old_leaf_id = match old_leaf_id {
        Some(id) => id,
        None => return CollectEntriesResult { entries: Vec::new(), common_ancestor_id: None },
    };

    let old_branch = get_branch(old_leaf_id);
    let old_path_ids: std::collections::HashSet<String> =
        old_branch.iter().map(|e| e.id().to_string()).collect();

    let target_path = get_branch(target_id);

    // Find deepest common ancestor (target_path is root-first)
    let mut common_ancestor_id: Option<String> = None;
    for entry in target_path.iter().rev() {
        if old_path_ids.contains(entry.id()) {
            common_ancestor_id = Some(entry.id().to_string());
            break;
        }
    }

    // Collect entries from old leaf back to common ancestor
    let mut entries: Vec<SessionEntry> = Vec::new();
    let mut current: Option<String> = Some(old_leaf_id.to_string());

    while let Some(ref cid) = current.clone() {
        if Some(cid.as_str()) == common_ancestor_id.as_deref() {
            break;
        }
        match get_entry(cid) {
            Some(entry) => {
                let parent_id = entry.parent_id().map(|s| s.to_string());
                entries.push(entry);
                current = parent_id;
            }
            None => break,
        }
    }

    entries.reverse();

    CollectEntriesResult {
        entries,
        common_ancestor_id,
    }
}

// ============================================================================
// Entry to Message Conversion
// ============================================================================

fn get_message_from_entry(entry: &SessionEntry) -> Option<Value> {
    match entry {
        SessionEntry::Message(e) => {
            let role = e.message.get("role").and_then(|r| r.as_str()).unwrap_or("");
            if role == "toolResult" {
                return None;
            }
            Some(e.message.clone())
        }
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
        // These don't contribute to conversation content
        SessionEntry::ThinkingLevelChange(_)
        | SessionEntry::ModelChange(_)
        | SessionEntry::Custom(_)
        | SessionEntry::Label(_)
        | SessionEntry::SessionInfo(_) => None,
    }
}

/// Prepare entries for summarization with token budget.
/// Walks from NEWEST to OLDEST, adding messages until token budget.
pub fn prepare_branch_entries(entries: &[SessionEntry], token_budget: u64) -> BranchPreparation {
    let mut messages: Vec<Value> = Vec::new();
    let mut file_ops = create_file_ops();
    let mut total_tokens = 0u64;

    // First pass: collect file ops from ALL entries
    for entry in entries {
        if let SessionEntry::BranchSummary(e) = entry {
            if e.from_hook != Some(true) {
                if let Some(details) = &e.details {
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

    // Second pass: walk from newest to oldest, adding messages until token budget
    for i in (0..entries.len()).rev() {
        let entry = &entries[i];
        let message = match get_message_from_entry(entry) {
            Some(m) => m,
            None => continue,
        };

        extract_file_ops_from_message(&message, &mut file_ops);
        let tokens = estimate_tokens(&message);

        if token_budget > 0 && total_tokens + tokens > token_budget {
            // If this is a summary entry, try to fit it if under 90% budget
            if matches!(entry, SessionEntry::Compaction(_) | SessionEntry::BranchSummary(_)) {
                if total_tokens < (token_budget as f64 * 0.9) as u64 {
                    messages.insert(0, message);
                    total_tokens += tokens;
                }
            }
            break;
        }

        messages.insert(0, message);
        total_tokens += tokens;
    }

    BranchPreparation {
        messages,
        file_ops,
        total_tokens,
    }
}

// ============================================================================
// Summary Generation
// ============================================================================

const BRANCH_SUMMARY_PREAMBLE: &str =
    "The user explored a different conversation branch before returning here.\n\
Summary of that exploration:\n\n";

const BRANCH_SUMMARY_PROMPT: &str = "Create a structured summary of this conversation branch \
for context when returning later.\n\n\
Use this EXACT format:\n\n\
## Goal\n\
[What was the user trying to accomplish in this branch?]\n\n\
## Constraints & Preferences\n\
- [Any constraints, preferences, or requirements mentioned]\n\
- [Or \"(none)\" if none were mentioned]\n\n\
## Progress\n\
### Done\n\
- [x] [Completed tasks/changes]\n\n\
### In Progress\n\
- [ ] [Work that was started but not finished]\n\n\
### Blocked\n\
- [Issues preventing progress, if any]\n\n\
## Key Decisions\n\
- **[Decision]**: [Brief rationale]\n\n\
## Next Steps\n\
1. [What should happen next to continue this work]\n\n\
Keep each section concise. Preserve exact file paths, function names, and error messages.";

/// Generate a summary of abandoned branch entries.
/// The caller provides a closure for the LLM call.
pub async fn generate_branch_summary<F, Fut>(
    entries: &[SessionEntry],
    model_context_window: u64,
    reserve_tokens: u64,
    custom_instructions: Option<&str>,
    replace_instructions: bool,
    call_llm: F,
) -> BranchSummaryResult
where
    F: Fn(String, Vec<Value>, u64) -> Fut,
    Fut: std::future::Future<Output = Result<(String, bool, bool), String>>,
{
    let token_budget = model_context_window.saturating_sub(reserve_tokens);
    let BranchPreparation { messages, file_ops, .. } =
        prepare_branch_entries(entries, token_budget);

    if messages.is_empty() {
        return BranchSummaryResult {
            summary: Some("No content to summarize".to_string()),
            read_files: None,
            modified_files: None,
            aborted: None,
            error: None,
        };
    }

    let conversation_text = serialize_conversation(&messages);

    let instructions: String = if replace_instructions {
        custom_instructions
            .unwrap_or(BRANCH_SUMMARY_PROMPT)
            .to_string()
    } else if let Some(ci) = custom_instructions {
        format!("{}\n\nAdditional focus: {}", BRANCH_SUMMARY_PROMPT, ci)
    } else {
        BRANCH_SUMMARY_PROMPT.to_string()
    };

    let prompt_text = format!(
        "<conversation>\n{}\n</conversation>\n\n{}",
        conversation_text, instructions
    );

    let summarization_messages = vec![serde_json::json!({
        "role": "user",
        "content": [{"type": "text", "text": prompt_text}],
        "timestamp": chrono::Utc::now().timestamp_millis()
    })];

    let result = call_llm(
        SUMMARIZATION_SYSTEM_PROMPT.to_string(),
        summarization_messages,
        2048,
    )
    .await;

    match result {
        Err(err) => BranchSummaryResult {
            summary: None,
            read_files: None,
            modified_files: None,
            aborted: None,
            error: Some(err),
        },
        Ok((text, aborted, _errored)) => {
            if aborted {
                return BranchSummaryResult {
                    summary: None,
                    read_files: None,
                    modified_files: None,
                    aborted: Some(true),
                    error: None,
                };
            }

            let summary = format!("{}{}", BRANCH_SUMMARY_PREAMBLE, text);
            let (read_files, modified_files) = compute_file_lists(&file_ops);
            let full_summary =
                format!("{}{}", summary, format_file_operations(&read_files, &modified_files));

            BranchSummaryResult {
                summary: Some(if full_summary.is_empty() {
                    "No summary generated".to_string()
                } else {
                    full_summary
                }),
                read_files: Some(read_files),
                modified_files: Some(modified_files),
                aborted: None,
                error: None,
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::session_manager::{BranchSummaryEntry, SessionEntry, SessionMessageEntry};

    fn make_user_entry(id: &str, parent_id: Option<&str>, text: &str) -> SessionEntry {
        SessionEntry::Message(SessionMessageEntry {
            entry_type: "message".to_string(),
            id: id.to_string(),
            parent_id: parent_id.map(|s| s.to_string()),
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            message: serde_json::json!({
                "role": "user",
                "content": text,
                "timestamp": 1
            }),
        })
    }

    fn make_assistant_entry(id: &str, parent_id: Option<&str>, text: &str) -> SessionEntry {
        SessionEntry::Message(SessionMessageEntry {
            entry_type: "message".to_string(),
            id: id.to_string(),
            parent_id: parent_id.map(|s| s.to_string()),
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            message: serde_json::json!({
                "role": "assistant",
                "content": [{"type": "text", "text": text}],
                "provider": "anthropic",
                "model": "claude-test",
                "timestamp": 1
            }),
        })
    }

    #[test]
    fn test_collect_entries_no_old_leaf() {
        let result = collect_entries_for_branch_summary(
            None,
            "target",
            |_| vec![],
            |_| None,
        );
        assert!(result.entries.is_empty());
        assert!(result.common_ancestor_id.is_none());
    }

    #[test]
    fn test_collect_entries_finds_common_ancestor() {
        // Tree: 1 -> 2 -> 3 (main), 2 -> 4 (branch)
        let entry1 = make_user_entry("1", None, "root");
        let entry2 = make_assistant_entry("2", Some("1"), "r1");
        let entry3 = make_user_entry("3", Some("2"), "branch A");
        let entry4 = make_user_entry("4", Some("2"), "branch B");

        let all = vec![entry1.clone(), entry2.clone(), entry3.clone(), entry4.clone()];

        let get_branch = |id: &str| -> Vec<SessionEntry> {
            // Walk from id to root
            let mut path = Vec::new();
            let mut current = Some(id.to_string());
            while let Some(ref cid) = current.clone() {
                if let Some(e) = all.iter().find(|e| e.id() == cid) {
                    path.push(e.clone());
                    current = e.parent_id().map(|s| s.to_string());
                } else {
                    break;
                }
            }
            path.reverse();
            path
        };

        let get_entry = |id: &str| -> Option<SessionEntry> {
            all.iter().find(|e| e.id() == id).cloned()
        };

        let result = collect_entries_for_branch_summary(
            Some("3"),
            "4",
            get_branch,
            get_entry,
        );

        // Common ancestor should be "2"
        assert_eq!(result.common_ancestor_id, Some("2".to_string()));
        // Entries from "3" back to "2" (not including "2")
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].id(), "3");
    }

    #[test]
    fn test_prepare_branch_entries_empty() {
        let result = prepare_branch_entries(&[], 0);
        assert!(result.messages.is_empty());
        assert_eq!(result.total_tokens, 0);
    }

    #[test]
    fn test_prepare_branch_entries_with_entries() {
        let entries = vec![
            make_user_entry("1", None, "hello"),
            make_assistant_entry("2", Some("1"), "hi there"),
        ];
        let result = prepare_branch_entries(&entries, 0); // 0 = no limit
        assert_eq!(result.messages.len(), 2);
    }

    #[test]
    fn test_prepare_branch_entries_respects_budget() {
        // Create many entries so budget forces truncation
        let ids: Vec<String> = (0..20).map(|i| format!("{}", i)).collect();
        let parent_ids: Vec<String> = (0usize..20).map(|i| format!("{}", i.saturating_sub(1))).collect();
        let contents: Vec<String> = (0..20).map(|_| "a".repeat(400)).collect();

        let entries: Vec<SessionEntry> = (0..20)
            .map(|i| {
                make_user_entry(
                    &ids[i],
                    if i == 0 { None } else { Some(&parent_ids[i - 1]) },
                    &contents[i],
                )
            })
            .collect();

        // Budget of 500 tokens should keep ~5 entries
        let result = prepare_branch_entries(&entries, 500);
        assert!(result.messages.len() < 20);
    }

    #[test]
    fn test_collect_file_ops_from_branch_summary_details() {
        // Branch summary with file tracking details
        let entry = SessionEntry::BranchSummary(BranchSummaryEntry {
            entry_type: "branch_summary".to_string(),
            id: "bs1".to_string(),
            parent_id: None,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            from_id: "root".to_string(),
            summary: "Some summary".to_string(),
            details: Some(serde_json::json!({
                "readFiles": ["a.rs", "b.rs"],
                "modifiedFiles": ["c.rs"]
            })),
            from_hook: None,
        });

        let result = prepare_branch_entries(&[entry], 0);
        assert!(result.file_ops.read.contains("a.rs"));
        assert!(result.file_ops.read.contains("b.rs"));
        assert!(result.file_ops.edited.contains("c.rs"));
    }
}
