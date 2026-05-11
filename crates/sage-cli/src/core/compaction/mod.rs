/// Compaction module — context compaction for long sessions.
///
/// Mirrors pi-mono packages/coding-agent/src/core/compaction/index.ts
pub mod branch_summarization;
#[allow(clippy::module_inception)]
pub mod compaction;
pub mod utils;

// Re-export commonly used items
