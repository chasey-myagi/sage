// Sprint 7 — S7.1: Session archival for wiki self-maintenance.
//
// Responsibilities:
// - Persist a completed `UserDriven` session's message history to
//   `<workspace>/raw/sessions/<session_id>.jsonl`.
// - List archived sessions for the daemon's IDLE-time maintenance check.
// - Count how many archived sessions the wiki hasn't processed yet by
//   diffing `raw/sessions/*.jsonl` against `wiki/log.md`'s processed-id set.
//
// JSONL format (frozen for this sprint):
//   Line 1: metadata record
//     { "version": 1, "session_id": "<id>", "session_type": "UserDriven",
//       "archived_at": <unix_ms> }
//   Line 2..N: one `AgentMessage` JSON per line, `\n`-terminated.
//
// Processed-session log format in `wiki/log.md`:
//   Each processed session MUST appear on its own line as
//       `- processed: <session_id>`
//   Lines that don't match are ignored. This keeps the log human-readable
//   while giving `count_unprocessed_sessions` a machine-grep target.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use sage_runner::config::SessionType;
use sage_runtime::types::AgentMessage;

/// Archive a completed session's message history to
/// `<workspace>/raw/sessions/<session_id>.jsonl`.
///
/// Only [`SessionType::UserDriven`] sessions are archived — the other
/// session types are internal (harness runs, wiki maintenance itself,
/// craft evaluation) and would pollute the "things to distill" bucket.
/// For non-`UserDriven` types the function returns `Ok(())` without
/// touching the filesystem.
///
/// On re-archive with the same `session_id` the existing file is
/// **overwritten** — it's the simpler contract and matches the "a
/// session is archived exactly once" expected flow; callers that need
/// the old copy should snapshot it first.
pub async fn archive_session(
    workspace_dir: &Path,
    session_id: &str,
    session_type: SessionType,
    messages: &[AgentMessage],
) -> anyhow::Result<()> {
    if session_type != SessionType::UserDriven {
        return Ok(());
    }

    let sessions_dir = workspace_dir.join("raw").join("sessions");
    tokio::fs::create_dir_all(&sessions_dir).await?;

    let now_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0) as u64;

    let metadata = serde_json::json!({
        "version": 1,
        "session_id": session_id,
        "session_type": session_type.archive_name(),
        "archived_at": now_unix_ms,
    });

    let mut content = serde_json::to_string(&metadata)?;
    content.push('\n');

    for msg in messages {
        content.push_str(&serde_json::to_string(msg)?);
        content.push('\n');
    }

    let path = sessions_dir.join(format!("{session_id}.jsonl"));
    let tmp = {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        path.with_extension(format!("jsonl.tmp.{pid}.{nanos}"))
    };
    tokio::fs::write(&tmp, content.as_bytes())
        .await
        .context("write temp archive")?;
    if let Err(e) = tokio::fs::rename(&tmp, &path).await {
        // Best-effort cleanup — stale tmp is cosmetic, not a correctness issue.
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(anyhow::Error::from(e).context("rename archive"));
    }

    Ok(())
}

/// List every `*.jsonl` file under `<workspace>/raw/sessions/`, sorted
/// ascending by mtime (oldest first).
///
/// Missing directory is not an error — returns an empty `Vec`.
/// Non-`*.jsonl` entries are ignored (e.g. the `.gitkeep` placeholder
/// left by `init_agent_at`).
pub async fn list_archived_sessions(workspace_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let sessions_dir = workspace_dir.join("raw").join("sessions");

    if !sessions_dir.exists() {
        return Ok(vec![]);
    }

    let mut entries: Vec<(PathBuf, SystemTime)> = vec![];

    let mut rd = tokio::fs::read_dir(&sessions_dir).await?;
    while let Some(entry) = rd.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let mtime = entry.metadata().await?.modified()?;
        entries.push((path, mtime));
    }

    entries.sort_by_key(|(_, mtime)| *mtime);

    Ok(entries.into_iter().map(|(p, _)| p).collect())
}

/// Count archived sessions that don't yet appear as `processed` in
/// `wiki/log.md`.
///
/// A session `<id>` is considered *processed* iff `wiki/log.md` contains
/// a line matching `- processed: <id>` (leading/trailing whitespace
/// tolerated). Missing `wiki/log.md` means nothing has been processed
/// yet → all archived sessions count as unprocessed.
pub async fn count_unprocessed_sessions(workspace_dir: &Path) -> anyhow::Result<usize> {
    let sessions_dir = workspace_dir.join("raw").join("sessions");
    if !sessions_dir.exists() {
        return Ok(0);
    }

    let log_path = workspace_dir.join("wiki").join("log.md");
    let processed: HashSet<String> = match tokio::fs::read_to_string(&log_path).await {
        Ok(content) => content
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                trimmed.strip_prefix("- processed:").map(|id| id.trim().to_string())
            })
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashSet::new(),
        Err(e) => return Err(e.into()),
    };

    let mut rd = tokio::fs::read_dir(&sessions_dir).await?;
    let mut count = 0usize;
    while let Some(entry) = rd.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            if !processed.contains(stem) {
                count += 1;
            }
        }
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sage_runtime::types::{AgentMessage, AssistantMessage, UserMessage};
    use std::time::Duration;

    fn sample_messages() -> Vec<AgentMessage> {
        vec![
            AgentMessage::User(UserMessage::from_text("hello")),
            AgentMessage::Assistant(AssistantMessage::new("hi there".into())),
        ]
    }

    // ── archive_session ────────────────────────────────────────────────────

    #[tokio::test]
    async fn archive_session_writes_jsonl_for_user_driven() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        tokio::fs::create_dir_all(ws.join("raw").join("sessions"))
            .await
            .unwrap();

        archive_session(ws, "sess-abc", SessionType::UserDriven, &sample_messages())
            .await
            .expect("archive_session should succeed");

        let expected = ws.join("raw").join("sessions").join("sess-abc.jsonl");
        assert!(expected.is_file(), "JSONL file must be created at {expected:?}");
    }

    #[tokio::test]
    async fn archive_session_skips_harness_run() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        tokio::fs::create_dir_all(ws.join("raw").join("sessions"))
            .await
            .unwrap();

        archive_session(ws, "sess-h", SessionType::HarnessRun, &sample_messages())
            .await
            .expect("non-UserDriven returns Ok without writing");

        let path = ws.join("raw").join("sessions").join("sess-h.jsonl");
        assert!(
            !path.exists(),
            "HarnessRun must not be archived, found file at {path:?}"
        );
    }

    #[tokio::test]
    async fn archive_session_skips_wiki_maintenance() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        archive_session(
            ws,
            "sess-w",
            SessionType::WikiMaintenance,
            &sample_messages(),
        )
        .await
        .expect("WikiMaintenance must not error");
        assert!(!ws.join("raw").join("sessions").join("sess-w.jsonl").exists());
    }

    #[tokio::test]
    async fn archive_session_skips_skill_evaluation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        archive_session(
            ws,
            "sess-c",
            SessionType::SkillEvaluation,
            &sample_messages(),
        )
        .await
        .expect("SkillEvaluation must not error");
        assert!(!ws.join("raw").join("sessions").join("sess-c.jsonl").exists());
    }

    #[tokio::test]
    async fn archive_session_first_line_is_metadata() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        archive_session(ws, "sess-meta", SessionType::UserDriven, &sample_messages())
            .await
            .unwrap();

        let path = ws.join("raw").join("sessions").join("sess-meta.jsonl");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let first_line = content.lines().next().expect("JSONL must have >=1 line");

        let meta: serde_json::Value = serde_json::from_str(first_line)
            .expect("first line must be valid JSON");
        assert_eq!(meta["version"], 1, "metadata version must be 1");
        assert_eq!(meta["session_id"], "sess-meta");
        assert_eq!(
            meta["session_type"], "UserDriven",
            "session_type must be rendered as PascalCase variant name"
        );
        assert!(
            meta["archived_at"].is_number(),
            "archived_at must be a numeric unix timestamp, got: {:?}",
            meta["archived_at"]
        );
    }

    #[tokio::test]
    async fn archive_session_subsequent_lines_are_agent_messages() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        let messages = sample_messages();
        archive_session(ws, "sess-msgs", SessionType::UserDriven, &messages)
            .await
            .unwrap();

        let path = ws.join("raw").join("sessions").join("sess-msgs.jsonl");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = content.lines().collect();

        assert_eq!(
            lines.len(),
            1 + messages.len(),
            "expected 1 metadata line + {} message lines, got {}",
            messages.len(),
            lines.len()
        );
        for line in lines.iter().skip(1) {
            let _: AgentMessage = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("line must round-trip as AgentMessage: {e} — line was: {line}"));
        }
    }

    #[tokio::test]
    async fn archive_session_creates_raw_sessions_dir_if_missing() {
        // Workspace exists but raw/sessions does not — archive_session must
        // create it rather than error out.
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();

        archive_session(ws, "sess-mkdir", SessionType::UserDriven, &sample_messages())
            .await
            .expect("must auto-create raw/sessions/");

        assert!(ws.join("raw").join("sessions").join("sess-mkdir.jsonl").is_file());
    }

    #[tokio::test]
    async fn archive_session_second_call_overwrites() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();

        archive_session(ws, "sess-dup", SessionType::UserDriven, &sample_messages())
            .await
            .unwrap();
        let path = ws.join("raw").join("sessions").join("sess-dup.jsonl");
        let first_len = tokio::fs::metadata(&path).await.unwrap().len();

        // Sleep briefly so mtime could differ; then re-archive with a
        // different (larger) message history.
        tokio::time::sleep(Duration::from_millis(10)).await;
        let mut big = sample_messages();
        big.push(AgentMessage::Assistant(AssistantMessage::new(
            "additional turn".into(),
        )));
        archive_session(ws, "sess-dup", SessionType::UserDriven, &big)
            .await
            .expect("second archive must succeed (overwrite semantics)");

        let second_len = tokio::fs::metadata(&path).await.unwrap().len();
        assert!(
            second_len > first_len,
            "overwritten file should reflect larger message history ({first_len} -> {second_len})"
        );
    }

    #[tokio::test]
    async fn archive_session_content_ends_with_newline() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        archive_session(ws, "sess-nl", SessionType::UserDriven, &sample_messages())
            .await
            .unwrap();

        let path = ws.join("raw").join("sessions").join("sess-nl.jsonl");
        let bytes = tokio::fs::read(&path).await.unwrap();
        assert!(bytes.ends_with(b"\n"), "archive file must end with a newline");
    }

    #[tokio::test]
    async fn archive_session_empty_messages_produces_single_line() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        archive_session(ws, "sess-empty", SessionType::UserDriven, &[])
            .await
            .unwrap();

        let path = ws.join("raw").join("sessions").join("sess-empty.jsonl");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1, "empty messages → only 1 metadata line, got {lines:?}");
        // The single line must be the metadata record.
        let meta: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(meta["session_type"], "UserDriven");
    }

    // ── list_archived_sessions ─────────────────────────────────────────────

    #[tokio::test]
    async fn list_archived_sessions_empty_dir_returns_empty_vec() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir_all(tmp.path().join("raw").join("sessions"))
            .await
            .unwrap();

        let got = list_archived_sessions(tmp.path()).await.unwrap();
        assert!(got.is_empty(), "empty dir must yield empty Vec, got {got:?}");
    }

    #[tokio::test]
    async fn list_archived_sessions_missing_dir_returns_empty_vec() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Deliberately do NOT create raw/sessions — function must tolerate it.
        let got = list_archived_sessions(tmp.path()).await.unwrap();
        assert!(got.is_empty(), "missing dir must yield empty Vec, got {got:?}");
    }

    #[tokio::test]
    async fn list_archived_sessions_orders_by_mtime_ascending() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();

        archive_session(ws, "sess-1-oldest", SessionType::UserDriven, &sample_messages())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        archive_session(ws, "sess-2-middle", SessionType::UserDriven, &sample_messages())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        archive_session(ws, "sess-3-newest", SessionType::UserDriven, &sample_messages())
            .await
            .unwrap();

        let got = list_archived_sessions(ws).await.unwrap();
        let names: Vec<String> = got
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![
                "sess-1-oldest.jsonl".to_string(),
                "sess-2-middle.jsonl".to_string(),
                "sess-3-newest.jsonl".to_string(),
            ],
            "files must be listed oldest-to-newest by mtime"
        );
    }

    #[tokio::test]
    async fn list_archived_sessions_ignores_non_jsonl_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        let sessions = ws.join("raw").join("sessions");
        tokio::fs::create_dir_all(&sessions).await.unwrap();

        // The .gitkeep and a stray README must not appear in the result.
        tokio::fs::write(sessions.join(".gitkeep"), b"").await.unwrap();
        tokio::fs::write(sessions.join("README.md"), b"not a session").await.unwrap();
        archive_session(ws, "sess-real", SessionType::UserDriven, &sample_messages())
            .await
            .unwrap();

        let got = list_archived_sessions(ws).await.unwrap();
        assert_eq!(got.len(), 1, "only the .jsonl file should count, got {got:?}");
        assert!(got[0].to_string_lossy().ends_with("sess-real.jsonl"));
    }

    // ── count_unprocessed_sessions ─────────────────────────────────────────

    async fn write_log(ws: &Path, body: &str) {
        let wiki = ws.join("wiki");
        tokio::fs::create_dir_all(&wiki).await.unwrap();
        tokio::fs::write(wiki.join("log.md"), body).await.unwrap();
    }

    #[tokio::test]
    async fn count_unprocessed_missing_log_counts_all() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        archive_session(ws, "s1", SessionType::UserDriven, &sample_messages())
            .await
            .unwrap();
        archive_session(ws, "s2", SessionType::UserDriven, &sample_messages())
            .await
            .unwrap();

        // No wiki/log.md at all → everything is unprocessed.
        let n = count_unprocessed_sessions(ws).await.unwrap();
        assert_eq!(n, 2, "missing log.md must mean all {n} sessions are unprocessed");
    }

    #[tokio::test]
    async fn count_unprocessed_empty_log_counts_all() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        archive_session(ws, "s1", SessionType::UserDriven, &sample_messages())
            .await
            .unwrap();
        archive_session(ws, "s2", SessionType::UserDriven, &sample_messages())
            .await
            .unwrap();
        write_log(ws, "# Wiki Maintenance Log\n\n<!-- nothing processed yet -->\n").await;

        let n = count_unprocessed_sessions(ws).await.unwrap();
        assert_eq!(n, 2);
    }

    #[tokio::test]
    async fn count_unprocessed_all_processed_returns_zero() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        archive_session(ws, "s1", SessionType::UserDriven, &sample_messages())
            .await
            .unwrap();
        archive_session(ws, "s2", SessionType::UserDriven, &sample_messages())
            .await
            .unwrap();
        write_log(
            ws,
            "# Wiki Maintenance Log\n\n- processed: s1\n- processed: s2\n",
        )
        .await;

        let n = count_unprocessed_sessions(ws).await.unwrap();
        assert_eq!(n, 0, "every archived session is listed in log → unprocessed=0");
    }

    #[tokio::test]
    async fn count_unprocessed_mixed_log_counts_remainder() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        for id in ["s1", "s2", "s3", "s4"] {
            archive_session(ws, id, SessionType::UserDriven, &sample_messages())
                .await
                .unwrap();
        }
        // Only s1 and s3 are processed; s2/s4 remain.
        write_log(
            ws,
            "# Wiki Log\n\nSome prose.\n- processed: s1\nmore prose.\n- processed: s3\n",
        )
        .await;

        let n = count_unprocessed_sessions(ws).await.unwrap();
        assert_eq!(n, 2, "s2 and s4 should still count as unprocessed");
    }

    #[tokio::test]
    async fn count_unprocessed_no_archive_dir_returns_zero() {
        let tmp = tempfile::TempDir::new().unwrap();
        // No raw/sessions and no wiki/log.md — nothing to do.
        let n = count_unprocessed_sessions(tmp.path()).await.unwrap();
        assert_eq!(n, 0);
    }
}
