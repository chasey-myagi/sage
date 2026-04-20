/// Compaction module — context compaction for long sessions.
///
/// Mirrors pi-mono packages/coding-agent/src/core/compaction/index.ts
pub mod branch_summarization;
pub mod compaction;
pub mod utils;

// Re-export commonly used items
pub use compaction::{
    CompactionDetails, CompactionPreparation, CompactionResult, CompactionSettings,
    ContextUsageEstimate, CutPointResult, DEFAULT_COMPACTION_SETTINGS, calculate_context_tokens,
    compact, estimate_context_tokens, estimate_tokens, find_cut_point, find_turn_start_index,
    get_last_assistant_usage, prepare_compaction, should_compact,
};

pub use branch_summarization::{
    BranchPreparation, BranchSummaryDetails, BranchSummaryResult, CollectEntriesResult,
    collect_entries_for_branch_summary, generate_branch_summary, prepare_branch_entries,
};

pub use utils::{
    FileOperations, SUMMARIZATION_SYSTEM_PROMPT, compute_file_lists, create_file_ops,
    extract_file_ops_from_message, format_file_operations, serialize_conversation,
};
