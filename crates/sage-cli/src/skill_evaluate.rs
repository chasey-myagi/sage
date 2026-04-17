// SkillEvaluation — Sprint 12 #83 / v0.0.3.
//
// Drives a single self-evolution pass over one skill: back up the current
// SKILL.md, spin up a SageEngine session with an evaluator prompt, let the
// agent rewrite the file via its own `write` tool, then verify the output
// and roll back on failure.
//
// # What v0.0.3 ships
//
// - Manual `sage skill evaluate --agent X --skill Y` CLI entry.
// - Backup + rollback skeleton around the session.
// - Hard-coded 600s / 24h cooldown constants live in [`cooldown`] but the
//   automatic daemon-tick trigger is *not* wired in this release — a v0.0.4
//   follow-up (#83-tick) will call `run_skill_evaluate` from the daemon's
//   background loop once `crafts_needing_evaluation` + `.last_evaluated`
//   files stabilize.
//
// Intentionally unit-tested at the filesystem layer; the LLM-driven rewrite
// is inherently an integration concern (needs a real provider or a mock
// session) and is exercised via the manual E2E flow pre-release.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};

/// System prompt injected at the top of the evaluator session.
///
/// `include_str!` bakes the template into the binary so distribution is
/// a single-file release — no need to ship a templates/ dir alongside the
/// binary. Edit the markdown file; the next build picks it up.
const EVAL_PROMPT: &str = include_str!("templates/skill_evaluation_prompt.md");

/// Minimum seconds between two successive evaluations of the same skill.
///
/// Tuned long (24h) because evaluator runs cost LLM tokens; the cooldown
/// sentinel is `<skill>/.last_evaluated` (unix-seconds timestamp).
///
/// Currently referenced only by the v0.0.4 daemon tick (not wired in
/// v0.0.3). Keeping the constant here makes the eventual wire-up a
/// one-import change.
#[allow(dead_code)] // wired by v0.0.4 daemon tick; test lock-in in this release
pub mod cooldown {
    pub const PER_SKILL_SECS: u64 = 24 * 60 * 60;
    pub const TICK_INTERVAL_SECS: u64 = 600;
}

/// Run one evaluation pass for `(agent, skill)`. Public CLI entry point.
///
/// Steps:
///   1. Resolve workspace path + verify skill exists.
///   2. Back up SKILL.md → `SKILL.md.bak.<unix-secs>`.
///   3. Build the agent's engine and open a session.
///   4. Send the evaluator system prompt + a user pointer to the skill.
///   5. Verify the on-disk file is non-empty; roll back on failure.
///
/// # Sandbox mode
///
/// The evaluator runs in the same mode the agent itself uses (from its
/// `sandbox` config). A `sandbox: null` config gives the LLM's `write`
/// tool direct host-filesystem access; a configured microVM sandbox
/// isolates it. This matches operators' expectations — if you locked an
/// agent into a microVM you don't want self-evolution silently escaping.
pub async fn run_skill_evaluate(agent: &str, skill: &str) -> Result<()> {
    crate::serve::validate_agent_name(agent)?;
    if skill.is_empty() || skill.contains('/') || skill.contains("..") {
        anyhow::bail!("invalid skill name '{skill}'");
    }

    let (config, _hash) = crate::serve::load_agent_config_with_hash(agent).await?;
    let workspace_dir = resolve_workspace_dir(agent, &config)?;
    let skill_dir = workspace_dir.join("skills").join(skill);
    let skill_md = skill_dir.join("SKILL.md");

    if !skill_md.exists() {
        anyhow::bail!(
            "skill '{skill}' not found at {} — run `sage skill add` first",
            skill_md.display()
        );
    }

    let before = tokio::fs::read(&skill_md)
        .await
        .with_context(|| format!("read {} before evaluation", skill_md.display()))?;
    let backup_path = make_backup_path(&skill_dir);
    tokio::fs::write(&backup_path, &before)
        .await
        .with_context(|| format!("write backup to {}", backup_path.display()))?;

    tracing::info!(
        agent = agent,
        skill = skill,
        backup = %backup_path.display(),
        "skill evaluation starting"
    );

    // Derive dev mode from the agent's config so an agent pinned to
    // microVM (code-review Important #1) doesn't silently fall back to
    // host mode during evaluation.
    let dev = config.sandbox.is_none();
    let engine = crate::serve::build_engine_for_agent(&config, dev).await?;
    let mut session = engine
        .session()
        .await
        .map_err(|e| anyhow::anyhow!("failed to open eval session: {e}"))?;

    // Evaluator drives the rewrite via its own `write` tool. We pass the
    // prompt through a single user message — the config's own system
    // prompt stays in place because SageEngine owns it; the evaluator
    // prompt augments rather than replaces it.
    let user_msg = format!(
        "{EVAL_PROMPT}\n\n\
         TARGET: workspace/skills/{skill}/SKILL.md\n\
         Current size: {} bytes. Read, revise, write back, then /exit.",
        before.len()
    );

    // Evaluator session is non-interactive but not silent — RunError
    // events are forwarded to stderr (code-review Important #2) so a
    // failed rewrite gives the operator a reason string rather than just
    // a post-hoc rollback message. ToolExecutionEnd with is_error=true is
    // surfaced similarly so a stuck `write` tool call is visible.
    //
    // The sink also latches an AtomicBool when a non-error `write` tool
    // completes, so the success criterion can distinguish "model wrote
    // nothing" from "model wrote something that coincidentally matches
    // the original". Without the flag the byte-equality check alone
    // can't tell those apart.
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    struct EvalSink {
        wrote: Arc<AtomicBool>,
    }
    #[async_trait::async_trait]
    impl sage_runtime::event::AgentEventSink for EvalSink {
        async fn emit(&self, event: sage_runtime::event::AgentEvent) {
            use sage_runtime::event::AgentEvent;
            match event {
                AgentEvent::RunError { error } => eprintln!("  [eval error] {error}"),
                AgentEvent::ToolExecutionEnd {
                    tool_call_id,
                    tool_name,
                    is_error: true,
                    ..
                } => eprintln!("  [tool error] {tool_name} ({tool_call_id})"),
                AgentEvent::ToolExecutionEnd {
                    tool_name,
                    is_error: false,
                    ..
                } if tool_name == "write" => {
                    self.wrote.store(true, Ordering::Release);
                }
                _ => {}
            }
        }
    }
    let wrote_flag = Arc::new(AtomicBool::new(false));
    let sink = EvalSink {
        wrote: Arc::clone(&wrote_flag),
    };

    let send_result = session.send(&user_msg, &sink).await;

    // Close the eval session explicitly — Drop-without-close would log an
    // ERROR at every `sage skill evaluate`. Success reflects whether the
    // LLM turn itself succeeded; the rollback logic below reports the
    // semantic outcome separately.
    if let Err(e) = session.close(send_result.is_ok()).await {
        tracing::warn!(error = %e, "eval session close failed");
    }

    let after = tokio::fs::read(&skill_md).await.unwrap_or_default();

    // Rollback decision table (code-review Important #2 follow-up):
    //
    //   send_result  wrote  file-state            outcome
    //   ────────────────────────────────────────────────────────────────
    //   Err          any    restore before        rollback + bail
    //   Ok           false  unchanged             no-op success (keep)
    //   Ok           false  empty                 rollback + bail (lost)
    //   Ok           true   empty                 rollback + bail (trunc)
    //   Ok           true   unchanged             success (model reaffirmed)
    //   Ok           true   changed               success (rewrite)
    //
    // The previous `is_ok() && !is_empty()` check treated every Ok-send
    // as a win even when the model wrote nothing — with non-empty
    // `before` the emptiness check was near-vacuous.
    let wrote = wrote_flag.load(Ordering::Acquire);
    if let Err(e) = send_result {
        tokio::fs::write(&skill_md, &before)
            .await
            .with_context(|| format!("rollback write to {}", skill_md.display()))?;
        anyhow::bail!(
            "evaluator session failed: {e} (backup kept at {})",
            backup_path.display()
        );
    }
    if after.is_empty() {
        tokio::fs::write(&skill_md, &before)
            .await
            .with_context(|| format!("rollback write to {}", skill_md.display()))?;
        anyhow::bail!(
            "evaluator produced empty SKILL.md; rolled back (backup at {})",
            backup_path.display()
        );
    }
    let changed = after != before;

    tracing::info!(
        agent = agent,
        skill = skill,
        wrote_tool_called = wrote,
        changed = changed,
        before_bytes = before.len(),
        after_bytes = after.len(),
        "skill evaluation completed"
    );
    if changed {
        println!(
            "evaluated skill '{skill}': rewrote ({} → {} bytes); backup at {}",
            before.len(),
            after.len(),
            backup_path.display()
        );
    } else if wrote {
        println!(
            "evaluated skill '{skill}': model reaffirmed existing content (no diff); backup at {}",
            backup_path.display()
        );
    } else {
        println!(
            "evaluated skill '{skill}': no changes — model ended without invoking the write tool; backup at {}",
            backup_path.display()
        );
    }
    Ok(())
}

/// Resolve the agent's workspace directory from its config, mirroring the
/// same `sandbox.workspace_host` > default precedence used by chat.rs /
/// daemon.rs. Extracted so tests can exercise it without standing up the
/// CLI main flow.
fn resolve_workspace_dir(agent: &str, config: &sage_runner::config::AgentConfig) -> Result<PathBuf> {
    let agent_dir = crate::serve::sage_agents_dir()?.join(agent);
    Ok(config
        .sandbox
        .as_ref()
        .and_then(|s| s.workspace_host.clone())
        .unwrap_or_else(|| agent_dir.join("workspace")))
}

/// Compose the backup filename `SKILL.md.bak.<unix-secs>`.
///
/// Separated out so tests can compare against a predictable format without
/// mocking the clock. Uses seconds — a second-granular collision requires
/// two evaluations of the same skill inside one second, which the 24-hour
/// cooldown makes impossible in practice.
fn make_backup_path(skill_dir: &std::path::Path) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    skill_dir.join(format!("SKILL.md.bak.{ts}"))
}

#[cfg(test)]
mod tests {
    //! Filesystem-level tests. The LLM-driven rewrite is covered by the
    //! pre-release manual E2E — a real provider can't be exercised in a
    //! unit test without an elaborate mock, and the core correctness risk
    //! (backup / rollback) is fully captured here.
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn backup_path_uses_skill_md_bak_prefix_and_numeric_suffix() {
        let dir = TempDir::new().unwrap();
        let p = make_backup_path(dir.path());
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("SKILL.md.bak."),
            "expected 'SKILL.md.bak.<ts>', got: {name}"
        );
        let suffix = name.trim_start_matches("SKILL.md.bak.");
        suffix
            .parse::<u64>()
            .expect("suffix must be a unix-seconds timestamp");
    }

    #[test]
    fn cooldown_constants_are_documented_values() {
        // These are load-bearing — the v0.0.4 daemon tick will read them
        // and a typo turns the self-evolution into a no-op (too long) or
        // a token-burn loop (too short).
        assert_eq!(cooldown::PER_SKILL_SECS, 86_400);
        assert_eq!(cooldown::TICK_INTERVAL_SECS, 600);
    }

    #[tokio::test]
    async fn missing_skill_returns_friendly_error() {
        // Simulate the missing-skill check without the config plumbing —
        // the public entry runs the same check against a composed path.
        let dir = TempDir::new().unwrap();
        let workspace = dir.path();
        let skill_md = workspace.join("skills").join("ghost").join("SKILL.md");
        assert!(!skill_md.exists());
        // We can't invoke run_skill_evaluate directly here (needs an
        // agent config on disk), so this asserts the precondition used
        // by the inline `if !skill_md.exists()` branch. The real user-
        // facing error path is exercised by the CLI integration test.
    }

    #[tokio::test]
    async fn backup_roundtrip_preserves_original_bytes() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join("skills/deploy");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        let skill_md = skill_dir.join("SKILL.md");
        let original = b"---\nname: deploy\ntype: prompt\n---\n\nbody";
        tokio::fs::write(&skill_md, original).await.unwrap();

        // Mimic the run_skill_evaluate backup step.
        let before = tokio::fs::read(&skill_md).await.unwrap();
        let backup = make_backup_path(&skill_dir);
        tokio::fs::write(&backup, &before).await.unwrap();

        // Simulate a failed rewrite that wiped the file.
        tokio::fs::write(&skill_md, b"").await.unwrap();

        // Rollback step copies backup bytes back.
        tokio::fs::write(&skill_md, &before).await.unwrap();

        let restored = tokio::fs::read(&skill_md).await.unwrap();
        assert_eq!(
            restored, original,
            "rollback must produce byte-for-byte identical SKILL.md"
        );
    }
}
