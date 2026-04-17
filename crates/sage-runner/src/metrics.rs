use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context as _;

use crate::config::SessionType;

const SUMMARY_MAX_ENTRIES: usize = 50;

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Write `bytes` to `dest` atomically: write to `<dest>.<unique>.tmp` then
/// rename onto `dest`. `rename(2)` on the same filesystem is atomic so
/// readers see either the old file or the fully-written new one.
///
/// `unique` disambiguates the tmp path so concurrent writers targeting the
/// same `dest` never trample each other's half-written bytes. Callers pass
/// the task_id (or any run-unique string) as `unique`.
async fn write_atomic(dest: &std::path::Path, bytes: &[u8], unique: &str) -> anyhow::Result<()> {
    let tmp = {
        let mut p = dest.as_os_str().to_owned();
        p.push(".");
        p.push(unique);
        p.push(".tmp");
        std::path::PathBuf::from(p)
    };
    tokio::fs::write(&tmp, bytes)
        .await
        .with_context(|| format!("write temp {}", tmp.display()))?;
    if let Err(e) = tokio::fs::rename(&tmp, dest).await {
        // Clean up the orphan tmp on rename failure. Best-effort — a stale
        // tmp is cosmetic, not a correctness issue.
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(anyhow::Error::from(e).context(format!(
            "rename {} → {}",
            tmp.display(),
            dest.display()
        )));
    }
    Ok(())
}

/// Task execution metrics collected during a single agent run.
///
/// Created at the start of each task and updated as `AgentEvent`s arrive.
/// On `UserDriven` sessions the finished record is written to
/// `<workspace>/metrics/<task_id>.json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskRecord {
    /// Unique identifier for this task run (ULID format).
    pub task_id: String,
    /// Agent name from `AgentConfig.name`.
    pub agent_name: String,
    /// LLM model identifier (e.g. `"claude-haiku-4-5-20251001"`).
    pub model: String,
    /// sha256 of the agent config bytes (provided externally).
    pub config_hash: String,
    /// Unix epoch milliseconds when the collector started observing.
    pub started_at: u64,
    /// Unix epoch milliseconds when the collector was finalized.
    pub ended_at: u64,
    /// `ended_at - started_at`.
    pub duration_ms: u64,
    /// Total LLM input tokens consumed across all turns.
    pub input_tokens: u64,
    /// Total LLM output tokens generated across all turns.
    pub output_tokens: u64,
    /// Total cache-read input tokens across all turns.
    pub cache_read_tokens: u64,
    /// Total cache-write input tokens across all turns.
    pub cache_write_tokens: u64,
    /// Number of completed turns (one per LLM response).
    pub turn_count: u32,
    /// Number of tool calls executed.
    pub tool_call_count: u32,
    /// Number of tool calls that reported `is_error = true`.
    pub tool_error_count: u32,
    /// Number of context compaction events (conversation history pruned).
    pub compaction_count: u32,
    /// Whether the task completed successfully (no error / abort).
    pub success: bool,
    /// Optional failure reason set by the caller at finalize time.
    pub failure_reason: Option<String>,
    /// Session classification — controls whether the record is persisted.
    pub session_type: SessionType,
    /// Craft slugs active during this session (Sprint 10+; empty today).
    pub crafts_active: Vec<String>,
}

impl TaskRecord {
    /// Create a new `TaskRecord` with a fresh ULID task ID and zeroed counters.
    ///
    /// Identity fields (`agent_name`, `model`, `config_hash`) are stored verbatim.
    /// `session_type` defaults to `UserDriven`; callers that need a different
    /// classification should construct the record through [`MetricsCollector::new`].
    pub fn new(agent_name: String, model: String) -> Self {
        Self {
            task_id: ulid::Ulid::new().to_string(),
            agent_name,
            model,
            config_hash: String::new(),
            started_at: 0,
            ended_at: 0,
            duration_ms: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            turn_count: 0,
            tool_call_count: 0,
            tool_error_count: 0,
            compaction_count: 0,
            success: false,
            failure_reason: None,
            session_type: SessionType::UserDriven,
            crafts_active: Vec::new(),
        }
    }
}

/// Rolling summary file persisted alongside per-task records.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct SummaryFile {
    records: Vec<TaskRecord>,
}

/// Accumulates `AgentEvent`s into a [`TaskRecord`] and, for `UserDriven` sessions,
/// persists the finished record to `<workspace>/metrics/`.
pub struct MetricsCollector {
    record: TaskRecord,
    workspace_dir: PathBuf,
}

impl MetricsCollector {
    /// Initialise a collector with identity + session classification.
    ///
    /// `started_at` is stamped immediately with the current unix-ms clock.
    pub fn new(
        agent_name: String,
        model: String,
        session_type: SessionType,
        workspace_dir: PathBuf,
        config_hash: String,
    ) -> Self {
        let mut record = TaskRecord::new(agent_name, model);
        record.session_type = session_type;
        record.config_hash = config_hash;
        record.started_at = now_unix_ms();
        Self {
            record,
            workspace_dir,
        }
    }

    /// Observe a single agent event and update counters in memory.
    ///
    /// Pure in-memory accumulation; no I/O. Unknown events are ignored
    /// without panicking so forward-compatible event variants stay safe.
    pub fn observe(&mut self, event: &sage_runtime::event::AgentEvent) {
        use sage_runtime::event::AgentEvent;
        match event {
            AgentEvent::TurnEnd { message, .. } => {
                self.record.input_tokens += message.usage.input;
                self.record.output_tokens += message.usage.output;
                self.record.cache_read_tokens += message.usage.cache_read;
                self.record.cache_write_tokens += message.usage.cache_write;
                self.record.turn_count += 1;
            }
            AgentEvent::ToolExecutionEnd { is_error, .. } => {
                self.record.tool_call_count += 1;
                if *is_error {
                    self.record.tool_error_count += 1;
                }
            }
            AgentEvent::CompactionEnd { .. } => {
                self.record.compaction_count += 1;
            }
            _ => {}
        }
    }

    /// Finalise the record.
    ///
    /// Only `SessionType::UserDriven` writes to disk:
    /// * `<workspace>/metrics/<task_id>.json`
    /// * `<workspace>/metrics/summary.json` (rolling 50, newest first)
    ///
    /// Other session types are returned without any filesystem side-effects.
    pub async fn finalize(
        mut self,
        success: bool,
        failure_reason: Option<String>,
    ) -> anyhow::Result<TaskRecord> {
        let ended_at = now_unix_ms();
        self.record.ended_at = ended_at;
        self.record.duration_ms = ended_at.saturating_sub(self.record.started_at);
        self.record.success = success;
        self.record.failure_reason = failure_reason;

        if self.record.session_type == SessionType::UserDriven {
            let metrics_dir = self.workspace_dir.join("metrics");
            tokio::fs::create_dir_all(&metrics_dir)
                .await
                .with_context(|| format!("create metrics dir at {}", metrics_dir.display()))?;

            let record_path = metrics_dir.join(format!("{}.json", self.record.task_id));
            let record_json =
                serde_json::to_vec_pretty(&self.record).context("serialize task record")?;
            write_atomic(&record_path, &record_json, &self.record.task_id).await?;

            let summary_path = metrics_dir.join("summary.json");
            // Best-effort read: any failure (missing, corrupt JSON, IO error)
            // falls back to empty. Summary is a rolling cache, not source of
            // truth — losing it at most costs history, not correctness.
            let mut records: Vec<TaskRecord> = tokio::fs::read(&summary_path)
                .await
                .ok()
                .and_then(|bytes| serde_json::from_slice::<SummaryFile>(&bytes).ok())
                .map(|s| s.records)
                .unwrap_or_default();
            records.insert(0, self.record.clone());
            records.truncate(SUMMARY_MAX_ENTRIES);
            let summary_json =
                serde_json::to_vec_pretty(&SummaryFile { records }).context("serialize summary")?;
            write_atomic(&summary_path, &summary_json, &self.record.task_id).await?;
        }

        Ok(self.record)
    }

    /// Borrow the current in-flight record.
    pub fn record(&self) -> &TaskRecord {
        &self.record
    }

    /// Mark a craft slug as active during this session.
    ///
    /// Called when the agent invokes a craft via slash-command or tool. The
    /// resulting `crafts_active` list feeds the offline efficiency scorer
    /// (Sprint 10 S10.3): `score = best_tokens / avg_tokens` across runs that
    /// exercised the same craft.
    ///
    /// Duplicates are silently ignored so callers can invoke this idempotently
    /// each time a craft is touched without filtering upstream.
    pub fn record_craft_used(&mut self, name: impl Into<String>) {
        let name = name.into();
        if !self.record.crafts_active.iter().any(|n| n == &name) {
            self.record.crafts_active.push(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::str::FromStr as _;
    use std::time::{SystemTime, UNIX_EPOCH};

    use sage_runtime::event::AgentEvent;
    use sage_runtime::types::{
        AgentMessage, AssistantMessage, Content, Cost, StopReason, ToolResultMessage, Usage,
    };
    use tempfile::TempDir;

    use super::*;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn is_ulid(s: &str) -> bool {
        ulid::Ulid::from_str(s).is_ok()
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn workspace() -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("create tempdir for workspace");
        let path = dir.path().to_path_buf();
        (dir, path)
    }

    fn make_collector(session_type: SessionType, workspace_dir: PathBuf) -> MetricsCollector {
        MetricsCollector::new(
            "feishu".to_string(),
            "claude-haiku-4-5-20251001".to_string(),
            session_type,
            workspace_dir,
            "cfg-hash-abc123".to_string(),
        )
    }

    fn turn_end_with_usage(
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
    ) -> AgentEvent {
        let assistant = AssistantMessage {
            content: vec![Content::Text {
                text: "response".into(),
            }],
            provider: "anthropic".into(),
            model: "claude-haiku-4-5-20251001".into(),
            usage: Usage {
                input,
                output,
                cache_read,
                cache_write,
                total_tokens: input + output + cache_read + cache_write,
                cost: Cost::default(),
            },
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };
        AgentEvent::TurnEnd {
            message: assistant,
            tool_results: vec![],
        }
    }

    fn tool_end(is_error: bool) -> AgentEvent {
        AgentEvent::ToolExecutionEnd {
            tool_call_id: "tc_1".into(),
            tool_name: "bash".into(),
            is_error,
        }
    }

    fn compaction_end() -> AgentEvent {
        AgentEvent::CompactionEnd {
            tokens_before: 10_000,
            messages_compacted: 12,
        }
    }

    fn read_summary(workspace_dir: &Path) -> serde_json::Value {
        let path = workspace_dir.join("metrics").join("summary.json");
        let bytes =
            std::fs::read(&path).unwrap_or_else(|e| panic!("read summary.json at {path:?}: {e}"));
        serde_json::from_slice(&bytes).expect("summary.json must be valid JSON")
    }

    // ── TaskRecord: legacy baseline (preserved from the thin version) ────────

    #[test]
    fn task_record_task_id_not_empty() {
        let rec = TaskRecord::new(
            "feishu".to_string(),
            "claude-haiku-4-5-20251001".to_string(),
        );
        assert!(!rec.task_id.is_empty());
    }

    #[test]
    fn task_record_task_id_is_ulid_format() {
        let rec = TaskRecord::new(
            "feishu".to_string(),
            "claude-haiku-4-5-20251001".to_string(),
        );
        assert!(
            is_ulid(&rec.task_id),
            "task_id '{}' is not a valid ULID (expected 26-char Crockford base32)",
            rec.task_id
        );
    }

    #[test]
    fn task_record_two_records_have_distinct_ids() {
        let a = TaskRecord::new("agent".to_string(), "model".to_string());
        let b = TaskRecord::new("agent".to_string(), "model".to_string());
        assert_ne!(
            a.task_id, b.task_id,
            "every TaskRecord must receive a unique ULID"
        );
    }

    #[test]
    fn task_record_new_stores_identity_and_zeros_counters() {
        let rec = TaskRecord::new(
            "feishu".to_string(),
            "claude-haiku-4-5-20251001".to_string(),
        );
        assert_eq!(rec.agent_name, "feishu");
        assert_eq!(rec.model, "claude-haiku-4-5-20251001");
        assert_eq!(rec.turn_count, 0);
        assert_eq!(rec.tool_call_count, 0);
        assert_eq!(rec.input_tokens, 0);
        assert_eq!(rec.output_tokens, 0);
        assert_eq!(rec.compaction_count, 0);
        assert!(
            !rec.success,
            "new TaskRecord must not be pre-marked as successful"
        );
    }

    #[test]
    fn task_record_empty_strings_produce_valid_ulid() {
        let rec = TaskRecord::new("".to_string(), "".to_string());
        assert!(
            is_ulid(&rec.task_id),
            "empty field inputs must still produce a valid ULID, got: '{}'",
            rec.task_id
        );
        assert_eq!(rec.agent_name, "");
        assert_eq!(rec.model, "");
    }

    #[test]
    fn task_record_unicode_agent_name() {
        let rec = TaskRecord::new("飞书助手".to_string(), "claude-haiku".to_string());
        assert!(is_ulid(&rec.task_id));
        assert_eq!(rec.agent_name, "飞书助手");
    }

    // ── TaskRecord: new fields + full serde ──────────────────────────────────

    #[test]
    fn task_record_new_task_id_is_ulid_and_unique() {
        let a = TaskRecord::new("agent".to_string(), "model".to_string());
        let b = TaskRecord::new("agent".to_string(), "model".to_string());
        assert!(is_ulid(&a.task_id));
        assert!(is_ulid(&b.task_id));
        assert_ne!(a.task_id, b.task_id);
    }

    #[test]
    fn task_record_new_extended_fields_default_to_zero_empty_or_none() {
        let rec = TaskRecord::new(
            "feishu".to_string(),
            "claude-haiku-4-5-20251001".to_string(),
        );
        assert_eq!(rec.config_hash, "");
        assert_eq!(rec.started_at, 0);
        assert_eq!(rec.ended_at, 0);
        assert_eq!(rec.duration_ms, 0);
        assert_eq!(rec.cache_read_tokens, 0);
        assert_eq!(rec.cache_write_tokens, 0);
        assert_eq!(rec.tool_error_count, 0);
        assert!(rec.failure_reason.is_none());
        assert_eq!(rec.session_type, SessionType::UserDriven);
        assert!(rec.crafts_active.is_empty());
    }

    #[test]
    fn task_record_new_zeros_legacy_counters() {
        let rec = TaskRecord::new(
            "feishu".to_string(),
            "claude-haiku-4-5-20251001".to_string(),
        );
        assert_eq!(rec.agent_name, "feishu");
        assert_eq!(rec.model, "claude-haiku-4-5-20251001");
        assert_eq!(rec.input_tokens, 0);
        assert_eq!(rec.output_tokens, 0);
        assert_eq!(rec.turn_count, 0);
        assert_eq!(rec.tool_call_count, 0);
        assert_eq!(rec.compaction_count, 0);
        assert!(!rec.success);
    }

    #[test]
    fn task_record_serde_roundtrip_preserves_all_fields() {
        let mut rec = TaskRecord::new(
            "feishu".to_string(),
            "claude-haiku-4-5-20251001".to_string(),
        );
        rec.config_hash = "deadbeef".into();
        rec.started_at = 1_700_000_000_000;
        rec.ended_at = 1_700_000_001_500;
        rec.duration_ms = 1_500;
        rec.input_tokens = 100;
        rec.output_tokens = 200;
        rec.cache_read_tokens = 50;
        rec.cache_write_tokens = 25;
        rec.turn_count = 3;
        rec.tool_call_count = 4;
        rec.tool_error_count = 1;
        rec.compaction_count = 2;
        rec.success = true;
        rec.failure_reason = Some("timeout".into());
        rec.session_type = SessionType::HarnessRun;
        rec.crafts_active = vec!["shell".into(), "docs".into()];

        let json = serde_json::to_string(&rec).expect("serialize TaskRecord");
        let back: TaskRecord = serde_json::from_str(&json).expect("deserialize TaskRecord");

        assert_eq!(back.task_id, rec.task_id);
        assert_eq!(back.agent_name, rec.agent_name);
        assert_eq!(back.model, rec.model);
        assert_eq!(back.config_hash, rec.config_hash);
        assert_eq!(back.started_at, rec.started_at);
        assert_eq!(back.ended_at, rec.ended_at);
        assert_eq!(back.duration_ms, rec.duration_ms);
        assert_eq!(back.input_tokens, rec.input_tokens);
        assert_eq!(back.output_tokens, rec.output_tokens);
        assert_eq!(back.cache_read_tokens, rec.cache_read_tokens);
        assert_eq!(back.cache_write_tokens, rec.cache_write_tokens);
        assert_eq!(back.turn_count, rec.turn_count);
        assert_eq!(back.tool_call_count, rec.tool_call_count);
        assert_eq!(back.tool_error_count, rec.tool_error_count);
        assert_eq!(back.compaction_count, rec.compaction_count);
        assert_eq!(back.success, rec.success);
        assert_eq!(back.failure_reason, rec.failure_reason);
        assert_eq!(back.session_type, rec.session_type);
        assert_eq!(back.crafts_active, rec.crafts_active);
    }

    // ── MetricsCollector::new ────────────────────────────────────────────────

    #[test]
    fn collector_new_stores_identity_session_type_and_config_hash() {
        let (_tmp, ws) = workspace();
        let collector = MetricsCollector::new(
            "feishu".into(),
            "claude-haiku-4-5-20251001".into(),
            SessionType::WikiMaintenance,
            ws,
            "cfg-hash-xyz".into(),
        );
        let rec = collector.record();
        assert_eq!(rec.agent_name, "feishu");
        assert_eq!(rec.model, "claude-haiku-4-5-20251001");
        assert_eq!(rec.session_type, SessionType::WikiMaintenance);
        assert_eq!(rec.config_hash, "cfg-hash-xyz");
    }

    #[test]
    fn collector_new_task_id_is_ulid() {
        let (_tmp, ws) = workspace();
        let collector = make_collector(SessionType::UserDriven, ws);
        assert!(is_ulid(&collector.record().task_id));
    }

    #[test]
    fn collector_new_sets_started_at_to_current_unix_ms() {
        let before = now_ms();
        let (_tmp, ws) = workspace();
        let collector = make_collector(SessionType::UserDriven, ws);
        let after = now_ms();
        let started = collector.record().started_at;
        assert!(started > 0, "started_at must be stamped (non-zero)");
        assert!(
            started >= before && started <= after,
            "started_at {started} must fall within [{before}, {after}]"
        );
    }

    #[test]
    fn collector_new_starts_with_zero_counters() {
        let (_tmp, ws) = workspace();
        let collector = make_collector(SessionType::UserDriven, ws);
        let rec = collector.record();
        assert_eq!(rec.input_tokens, 0);
        assert_eq!(rec.output_tokens, 0);
        assert_eq!(rec.cache_read_tokens, 0);
        assert_eq!(rec.cache_write_tokens, 0);
        assert_eq!(rec.turn_count, 0);
        assert_eq!(rec.tool_call_count, 0);
        assert_eq!(rec.tool_error_count, 0);
        assert_eq!(rec.compaction_count, 0);
    }

    // ── observe: accumulation rules ──────────────────────────────────────────

    #[test]
    fn observe_turn_end_accumulates_all_token_fields() {
        let (_tmp, ws) = workspace();
        let mut collector = make_collector(SessionType::UserDriven, ws);
        collector.observe(&turn_end_with_usage(100, 50, 10, 5));
        let rec = collector.record();
        assert_eq!(rec.input_tokens, 100);
        assert_eq!(rec.output_tokens, 50);
        assert_eq!(rec.cache_read_tokens, 10);
        assert_eq!(rec.cache_write_tokens, 5);
        assert_eq!(rec.turn_count, 1);
    }

    #[test]
    fn observe_multiple_turn_ends_sums_tokens_and_increments_turn_count() {
        let (_tmp, ws) = workspace();
        let mut collector = make_collector(SessionType::UserDriven, ws);
        collector.observe(&turn_end_with_usage(100, 50, 10, 5));
        collector.observe(&turn_end_with_usage(200, 100, 20, 0));
        collector.observe(&turn_end_with_usage(0, 0, 0, 0));
        let rec = collector.record();
        assert_eq!(rec.input_tokens, 300);
        assert_eq!(rec.output_tokens, 150);
        assert_eq!(rec.cache_read_tokens, 30);
        assert_eq!(rec.cache_write_tokens, 5);
        assert_eq!(rec.turn_count, 3);
    }

    #[test]
    fn observe_tool_execution_end_success_increments_only_call_count() {
        let (_tmp, ws) = workspace();
        let mut collector = make_collector(SessionType::UserDriven, ws);
        collector.observe(&tool_end(false));
        let rec = collector.record();
        assert_eq!(rec.tool_call_count, 1);
        assert_eq!(rec.tool_error_count, 0);
    }

    #[test]
    fn observe_tool_execution_end_error_increments_both_counters() {
        let (_tmp, ws) = workspace();
        let mut collector = make_collector(SessionType::UserDriven, ws);
        collector.observe(&tool_end(true));
        let rec = collector.record();
        assert_eq!(rec.tool_call_count, 1);
        assert_eq!(rec.tool_error_count, 1);
    }

    #[test]
    fn observe_compaction_end_increments_compaction_count() {
        let (_tmp, ws) = workspace();
        let mut collector = make_collector(SessionType::UserDriven, ws);
        collector.observe(&compaction_end());
        collector.observe(&compaction_end());
        assert_eq!(collector.record().compaction_count, 2);
    }

    #[test]
    fn observe_ignores_agent_start_turn_start_and_message_update() {
        let (_tmp, ws) = workspace();
        let mut collector = make_collector(SessionType::UserDriven, ws);
        let initial = collector.record().clone();
        collector.observe(&AgentEvent::AgentStart);
        collector.observe(&AgentEvent::TurnStart);
        collector.observe(&AgentEvent::MessageUpdate {
            message: AgentMessage::assistant("hi".into()),
            delta: "hi".into(),
        });
        collector.observe(&AgentEvent::CompactionStart {
            reason: "token limit".into(),
            message_count: 10,
        });
        collector.observe(&AgentEvent::ToolExecutionStart {
            tool_call_id: "tc_1".into(),
            tool_name: "bash".into(),
            args: serde_json::json!({}),
        });
        collector.observe(&AgentEvent::AgentEnd { messages: vec![] });

        let rec = collector.record();
        assert_eq!(rec.input_tokens, initial.input_tokens);
        assert_eq!(rec.output_tokens, initial.output_tokens);
        assert_eq!(rec.cache_read_tokens, initial.cache_read_tokens);
        assert_eq!(rec.cache_write_tokens, initial.cache_write_tokens);
        assert_eq!(rec.turn_count, initial.turn_count);
        assert_eq!(rec.tool_call_count, initial.tool_call_count);
        assert_eq!(rec.tool_error_count, initial.tool_error_count);
        assert_eq!(rec.compaction_count, initial.compaction_count);
    }

    #[test]
    fn observe_mixed_events_accumulate_correctly() {
        let (_tmp, ws) = workspace();
        let mut collector = make_collector(SessionType::UserDriven, ws);
        collector.observe(&turn_end_with_usage(10, 5, 1, 1));
        collector.observe(&tool_end(false));
        collector.observe(&turn_end_with_usage(20, 15, 2, 0));
        collector.observe(&tool_end(true));
        collector.observe(&turn_end_with_usage(30, 25, 0, 3));
        collector.observe(&compaction_end());

        let rec = collector.record();
        assert_eq!(rec.input_tokens, 60);
        assert_eq!(rec.output_tokens, 45);
        assert_eq!(rec.cache_read_tokens, 3);
        assert_eq!(rec.cache_write_tokens, 4);
        assert_eq!(rec.turn_count, 3);
        assert_eq!(rec.tool_call_count, 2);
        assert_eq!(rec.tool_error_count, 1);
        assert_eq!(rec.compaction_count, 1);
    }

    #[test]
    fn observe_turn_end_with_tool_results_still_counts_turn_not_tools() {
        // TurnEnd carries tool_results, but only ToolExecutionEnd bumps tool counters.
        let (_tmp, ws) = workspace();
        let mut collector = make_collector(SessionType::UserDriven, ws);

        let assistant = AssistantMessage {
            content: vec![Content::Text { text: "ok".into() }],
            provider: "anthropic".into(),
            model: "claude".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };
        let tool_result = ToolResultMessage {
            tool_call_id: "tc_1".into(),
            tool_name: "bash".into(),
            content: vec![Content::Text { text: "out".into() }],
            is_error: true,
            timestamp: 0,
        };
        collector.observe(&AgentEvent::TurnEnd {
            message: assistant,
            tool_results: vec![tool_result],
        });

        let rec = collector.record();
        assert_eq!(rec.turn_count, 1);
        assert_eq!(rec.tool_call_count, 0);
        assert_eq!(rec.tool_error_count, 0);
    }

    // ── finalize: common behaviour ───────────────────────────────────────────

    #[tokio::test]
    async fn finalize_sets_success_true_and_clears_failure_reason() {
        let (_tmp, ws) = workspace();
        let collector = make_collector(SessionType::UserDriven, ws);
        let rec = collector.finalize(true, None).await.expect("finalize");
        assert!(rec.success);
        assert!(rec.failure_reason.is_none());
    }

    #[tokio::test]
    async fn finalize_sets_success_false_and_records_failure_reason() {
        let (_tmp, ws) = workspace();
        let collector = make_collector(SessionType::UserDriven, ws);
        let rec = collector
            .finalize(false, Some("llm timeout".into()))
            .await
            .expect("finalize");
        assert!(!rec.success);
        assert_eq!(rec.failure_reason.as_deref(), Some("llm timeout"));
    }

    #[tokio::test]
    async fn finalize_stamps_ended_at_and_duration_ms() {
        let (_tmp, ws) = workspace();
        let collector = make_collector(SessionType::UserDriven, ws);
        let started = collector.record().started_at;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let rec = collector.finalize(true, None).await.expect("finalize");
        assert!(
            rec.ended_at >= started,
            "ended_at {} must be >= started_at {started}",
            rec.ended_at
        );
        assert_eq!(
            rec.duration_ms,
            rec.ended_at - rec.started_at,
            "duration_ms must equal ended_at - started_at"
        );
    }

    #[tokio::test]
    async fn finalize_preserves_config_hash_verbatim() {
        let (_tmp, ws) = workspace();
        let collector = MetricsCollector::new(
            "feishu".into(),
            "claude".into(),
            SessionType::UserDriven,
            ws,
            "verbatim-hash-9876".into(),
        );
        let rec = collector.finalize(true, None).await.expect("finalize");
        assert_eq!(rec.config_hash, "verbatim-hash-9876");
    }

    // ── finalize: UserDriven writes record + summary ─────────────────────────

    #[tokio::test]
    async fn finalize_user_driven_writes_task_record_json() {
        let (_tmp, ws) = workspace();
        let collector = make_collector(SessionType::UserDriven, ws.clone());
        let rec = collector.finalize(true, None).await.expect("finalize");
        let path = ws.join("metrics").join(format!("{}.json", rec.task_id));
        assert!(path.is_file(), "expected {} to exist", path.display());
    }

    #[tokio::test]
    async fn finalize_user_driven_record_json_roundtrip_equals_memory() {
        let (_tmp, ws) = workspace();
        let mut collector = make_collector(SessionType::UserDriven, ws.clone());
        collector.observe(&turn_end_with_usage(42, 21, 3, 2));
        collector.observe(&tool_end(true));
        let rec = collector
            .finalize(false, Some("boom".into()))
            .await
            .expect("finalize");

        let path = ws.join("metrics").join(format!("{}.json", rec.task_id));
        let bytes = std::fs::read(&path).expect("read record file");
        let parsed: TaskRecord = serde_json::from_slice(&bytes).expect("parse record JSON");
        assert_eq!(parsed.task_id, rec.task_id);
        assert_eq!(parsed.input_tokens, 42);
        assert_eq!(parsed.output_tokens, 21);
        assert_eq!(parsed.cache_read_tokens, 3);
        assert_eq!(parsed.cache_write_tokens, 2);
        assert_eq!(parsed.tool_call_count, 1);
        assert_eq!(parsed.tool_error_count, 1);
        assert!(!parsed.success);
        assert_eq!(parsed.failure_reason.as_deref(), Some("boom"));
        assert_eq!(parsed.session_type, SessionType::UserDriven);
    }

    #[tokio::test]
    async fn finalize_user_driven_creates_metrics_directory_when_missing() {
        let (_tmp, ws) = workspace();
        assert!(!ws.join("metrics").exists(), "precondition");
        let collector = make_collector(SessionType::UserDriven, ws.clone());
        let _ = collector.finalize(true, None).await.expect("finalize");
        assert!(ws.join("metrics").is_dir(), "metrics/ must be created");
    }

    // ── finalize: non-UserDriven skip persistence ────────────────────────────

    #[tokio::test]
    async fn finalize_harness_run_does_not_write_any_files() {
        let (_tmp, ws) = workspace();
        let collector = make_collector(SessionType::HarnessRun, ws.clone());
        let rec = collector.finalize(true, None).await.expect("finalize");
        assert_eq!(rec.session_type, SessionType::HarnessRun);
        let metrics_dir = ws.join("metrics");
        assert!(
            !metrics_dir.exists(),
            "HarnessRun must not create metrics/ (got: {})",
            metrics_dir.display()
        );
    }

    #[tokio::test]
    async fn finalize_wiki_maintenance_does_not_write_any_files() {
        let (_tmp, ws) = workspace();
        let collector = make_collector(SessionType::WikiMaintenance, ws.clone());
        let _ = collector.finalize(true, None).await.expect("finalize");
        assert!(
            !ws.join("metrics").exists(),
            "WikiMaintenance must not create metrics/"
        );
    }

    #[tokio::test]
    async fn finalize_craft_evaluation_does_not_write_any_files() {
        let (_tmp, ws) = workspace();
        let collector = make_collector(SessionType::CraftEvaluation, ws.clone());
        let _ = collector.finalize(true, None).await.expect("finalize");
        assert!(
            !ws.join("metrics").exists(),
            "CraftEvaluation must not create metrics/"
        );
    }

    // ── summary.json lifecycle ───────────────────────────────────────────────

    #[tokio::test]
    async fn summary_created_on_first_user_driven_finalize() {
        let (_tmp, ws) = workspace();
        let collector = make_collector(SessionType::UserDriven, ws.clone());
        let rec = collector.finalize(true, None).await.expect("finalize");

        let value = read_summary(&ws);
        let records = value
            .get("records")
            .and_then(|v| v.as_array())
            .expect("summary.records must be an array");
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].get("task_id").and_then(|v| v.as_str()),
            Some(rec.task_id.as_str())
        );
    }

    #[tokio::test]
    async fn summary_prepends_newest_record_on_second_finalize() {
        let (_tmp, ws) = workspace();

        let first = make_collector(SessionType::UserDriven, ws.clone())
            .finalize(true, None)
            .await
            .expect("finalize 1");
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let second = make_collector(SessionType::UserDriven, ws.clone())
            .finalize(true, None)
            .await
            .expect("finalize 2");

        let value = read_summary(&ws);
        let records = value
            .get("records")
            .and_then(|v| v.as_array())
            .expect("records array");
        assert_eq!(records.len(), 2);
        assert_eq!(
            records[0].get("task_id").and_then(|v| v.as_str()),
            Some(second.task_id.as_str()),
            "newest must be at index 0"
        );
        assert_eq!(
            records[1].get("task_id").and_then(|v| v.as_str()),
            Some(first.task_id.as_str())
        );
    }

    #[tokio::test]
    async fn summary_truncates_to_fifty_newest_records() {
        let (_tmp, ws) = workspace();
        let mut task_ids = Vec::new();
        for _ in 0..51 {
            let rec = make_collector(SessionType::UserDriven, ws.clone())
                .finalize(true, None)
                .await
                .expect("finalize");
            task_ids.push(rec.task_id);
        }

        let value = read_summary(&ws);
        let records = value
            .get("records")
            .and_then(|v| v.as_array())
            .expect("records array");
        assert_eq!(records.len(), 50, "summary must cap at 50 entries");

        let newest = task_ids.last().expect("at least one run");
        assert_eq!(
            records[0].get("task_id").and_then(|v| v.as_str()),
            Some(newest.as_str()),
            "index 0 must be the most recent task_id"
        );

        let oldest_kept = &task_ids[1];
        assert_eq!(
            records[49].get("task_id").and_then(|v| v.as_str()),
            Some(oldest_kept.as_str()),
            "tail entry must correspond to the 2nd finalize (1st was evicted)"
        );

        let evicted = &task_ids[0];
        let retained: Vec<&str> = records
            .iter()
            .filter_map(|v| v.get("task_id").and_then(|t| t.as_str()))
            .collect();
        assert!(
            !retained.iter().any(|id| *id == evicted.as_str()),
            "the oldest task_id must have been evicted"
        );
    }

    #[tokio::test]
    async fn summary_with_corrupt_json_is_overwritten_with_current_record_only() {
        let (_tmp, ws) = workspace();
        let metrics_dir = ws.join("metrics");
        std::fs::create_dir_all(&metrics_dir).unwrap();
        std::fs::write(metrics_dir.join("summary.json"), b"not-json").unwrap();

        let rec = make_collector(SessionType::UserDriven, ws.clone())
            .finalize(true, None)
            .await
            .expect("finalize must not fail on corrupt summary");

        let value = read_summary(&ws);
        let records = value
            .get("records")
            .and_then(|v| v.as_array())
            .expect("records array");
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].get("task_id").and_then(|v| v.as_str()),
            Some(rec.task_id.as_str())
        );
    }

    #[tokio::test]
    async fn summary_is_not_written_for_non_user_driven_session() {
        let (_tmp, ws) = workspace();
        let _ = make_collector(SessionType::HarnessRun, ws.clone())
            .finalize(true, None)
            .await
            .expect("finalize");
        assert!(
            !ws.join("metrics").join("summary.json").exists(),
            "HarnessRun must not touch summary.json"
        );
    }

    // ── Sprint 10 S10.3 — record_craft_used ─────────────────────────────────

    #[test]
    fn record_craft_used_appends_fresh_slug() {
        let tmp = TempDir::new().unwrap();
        let mut c = MetricsCollector::new(
            "a".into(),
            "m".into(),
            SessionType::UserDriven,
            tmp.path().into(),
            String::new(),
        );
        c.record_craft_used("deploy-rune");
        assert_eq!(c.record().crafts_active, vec!["deploy-rune".to_string()]);
    }

    #[test]
    fn record_craft_used_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let mut c = MetricsCollector::new(
            "a".into(),
            "m".into(),
            SessionType::UserDriven,
            tmp.path().into(),
            String::new(),
        );
        c.record_craft_used("deploy-rune");
        c.record_craft_used("deploy-rune");
        c.record_craft_used("deploy-rune");
        assert_eq!(
            c.record().crafts_active.len(),
            1,
            "duplicate craft slug must not be appended twice"
        );
    }

    #[test]
    fn record_craft_used_preserves_insertion_order_for_distinct_slugs() {
        let tmp = TempDir::new().unwrap();
        let mut c = MetricsCollector::new(
            "a".into(),
            "m".into(),
            SessionType::UserDriven,
            tmp.path().into(),
            String::new(),
        );
        c.record_craft_used("a");
        c.record_craft_used("b");
        c.record_craft_used("c");
        c.record_craft_used("a"); // dup, still at head
        assert_eq!(
            c.record().crafts_active,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn record_craft_used_accepts_string_and_str() {
        let tmp = TempDir::new().unwrap();
        let mut c = MetricsCollector::new(
            "a".into(),
            "m".into(),
            SessionType::UserDriven,
            tmp.path().into(),
            String::new(),
        );
        c.record_craft_used("literal");
        c.record_craft_used(String::from("owned"));
        assert_eq!(c.record().crafts_active.len(), 2);
    }

    #[tokio::test]
    async fn crafts_active_persists_into_finalized_task_record_file() {
        let tmp = TempDir::new().unwrap();
        let ws: PathBuf = tmp.path().into();
        let mut c = MetricsCollector::new(
            "a".into(),
            "m".into(),
            SessionType::UserDriven,
            ws.clone(),
            String::new(),
        );
        c.record_craft_used("deploy-rune");
        c.record_craft_used("review-diff");
        let _rec = c.finalize(true, None).await.unwrap();

        let metrics_dir = ws.join("metrics");
        let files: Vec<_> = std::fs::read_dir(&metrics_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let n = e.file_name();
                let s = n.to_string_lossy();
                s.ends_with(".json") && !s.starts_with("summary")
            })
            .collect();
        assert_eq!(files.len(), 1, "expected exactly one task record file");

        let content = std::fs::read_to_string(files[0].path()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let crafts = parsed["crafts_active"].as_array().unwrap();
        let names: Vec<&str> = crafts.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(names, vec!["deploy-rune", "review-diff"]);
    }
}
