// Chat TUI — interactive foreground session with a SageSession.
//
// Task #88: `/slash` skill invocation removed. Agents discover and load
// skills on their own via `workspace/skills/INDEX.md` + Read tool; the TUI
// no longer dispatches `/<name>` as a skill shortcut. Only two built-ins
// remain (`/exit`, `/reset`).

/// Chat input parsed from the user's TUI prompt.
#[derive(Debug, PartialEq)]
pub enum ChatInput {
    /// A regular message to forward to the agent. Any line beginning with `/`
    /// that isn't `/exit` or `/reset` flows here verbatim — the agent decides
    /// whether to treat it as a command.
    Message(String),
    /// The user typed `/exit` — close the session.
    Exit,
    /// The user typed `/reset` — clear conversation history and start fresh.
    Reset,
    /// Empty or whitespace-only input — nothing to do.
    Empty,
}

/// Parse a raw string from the TUI input line into a [`ChatInput`].
///
/// Rules (applied to the trimmed value):
/// - `/exit` → [`ChatInput::Exit`] (case-insensitive, must stand alone)
/// - `/reset` → [`ChatInput::Reset`] (case-insensitive, must stand alone)
/// - Whitespace-only → [`ChatInput::Empty`]
/// - Everything else → [`ChatInput::Message`] with the **original** input preserved
pub fn parse_user_input(input: &str) -> ChatInput {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return ChatInput::Empty;
    }
    if trimmed.eq_ignore_ascii_case("/exit") {
        return ChatInput::Exit;
    }
    if trimmed.eq_ignore_ascii_case("/reset") {
        return ChatInput::Reset;
    }
    ChatInput::Message(input.to_string())
}

// ── TerminalSink ─────────────────────────────────────────────────────

/// An [`AgentEventSink`] that prints events to stdout/stderr in real time.
struct TerminalSink;

#[async_trait::async_trait]
impl sage_runtime::event::AgentEventSink for TerminalSink {
    async fn emit(&self, event: sage_runtime::event::AgentEvent) {
        use sage_runtime::event::AgentEvent;
        match &event {
            AgentEvent::MessageUpdate { delta, .. } => {
                use std::io::Write;
                print!("{delta}");
                let _ = std::io::stdout().flush();
            }
            AgentEvent::ToolExecutionStart { tool_name, .. } => {
                eprintln!("\n  [tool: {tool_name}]");
            }
            AgentEvent::ToolExecutionEnd { tool_name, is_error, .. } => {
                if *is_error {
                    eprintln!("  [tool: {tool_name} — ERROR]");
                }
            }
            AgentEvent::CompactionStart { reason, message_count } => {
                eprintln!("\n  [compacting: {reason}, {message_count} messages...]");
            }
            AgentEvent::CompactionEnd { messages_compacted, .. } => {
                eprintln!("  [compacted {messages_compacted} messages]");
            }
            AgentEvent::RunError { error } => {
                eprintln!("\nError: {error}");
            }
            _ => {}
        }
    }
}

/// Run an interactive foreground chat session with the named agent.
///
/// Loads the agent config, builds a [`SageSession`], and enters a read-eval loop.
/// Built-in slash commands: `/exit` to quit, `/reset` to clear history.
pub async fn run_chat(agent: &str, dev: bool) -> anyhow::Result<()> {
    // Task #85 defense-in-depth. `load_agent_config` also validates, but
    // running the check at every CLI entry point means a malicious
    // `--agent ../foo` can never reach `sage_agents_dir().join(agent)`
    // via an unvalidated path. The cost is one comparison — negligible.
    crate::serve::validate_agent_name(agent)?;
    // Task #80: pull config + sha256 hash together so TaskRecord.config_hash
    // gets a real `"sha256:<hex>"` instead of the old `""` sentinel.
    let (config, config_hash) = crate::serve::load_agent_config_with_hash(agent).await?;
    let engine = crate::serve::build_engine_for_agent(&config, dev).await?;

    // Sprint 11 #56 + Sprint 12 task #69: the engine's cancel token is now
    // shared with the SageSession and threaded into `run_agent_loop` via
    // tokio::select! checkpoints. Ctrl+C at the readline boundary shuts the
    // chat loop; Ctrl+C during a `session.send()` aborts the in-flight LLM
    // call / tool execution and returns `AgentLoopError::Cancelled`.
    let cancel_token = engine.cancel_token().clone();

    let mut session = engine
        .session()
        .await
        .map_err(|e| anyhow::anyhow!("failed to start session: {e}"))?;

    // Sprint 12 task #75: stand up the metrics pipeline. MetricsCollector
    // accumulates AgentEvents into a TaskRecord, written to
    // `<workspace>/metrics/<task_id>.json` + summary.json at session end
    // (UserDriven sessions only). TerminalSink is wrapped in MetricsSink so
    // the tee happens transparently inside session.send().
    let agent_dir = crate::serve::sage_agents_dir()?.join(agent);
    let workspace_dir = config
        .sandbox
        .as_ref()
        .and_then(|s| s.workspace_host.clone())
        .unwrap_or_else(|| agent_dir.join("workspace"));
    let session_type = config
        .memory
        .as_ref()
        .and_then(|m| m.session_type.clone())
        .unwrap_or(sage_runner::config::SessionType::UserDriven);
    let collector = sage_runner::metrics::MetricsCollector::new(
        config.name.clone(),
        config.llm.provider.clone(),
        config.llm.model.clone(),
        session_type,
        workspace_dir,
        config_hash,
    );
    let shared_metrics = sage_runner::metrics::share_collector(collector);
    let sink = sage_runner::metrics::MetricsSink::new(shared_metrics.clone(), TerminalSink);

    // Sprint 12 task #75 (Linus v1 blocker): run the chat loop inside a
    // separate async fn whose result we always observe before finalizing.
    // Previously `send_with_cancel(...).await?` would bubble anyhow::Err
    // out of run_chat and skip finalize entirely, silently losing the
    // TaskRecord on every failed turn. Now the Result flows here and the
    // metrics record accurately reflects success vs. the failure reason.
    let loop_result = chat_loop(
        agent,
        &agent_dir,
        &config.llm.provider,
        &config.llm.model,
        &mut session,
        &sink,
        &cancel_token,
    )
    .await;

    let (success, failure_reason) = match &loop_result {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };
    if let Some(collector) = sage_runner::metrics::take_collector(&shared_metrics).await {
        if let Err(e) = collector.finalize(success, failure_reason).await {
            tracing::warn!(error = %e, "metrics finalize failed at chat close");
        }
    }
    // Explicit SageSession teardown so Drop doesn't synthesize a
    // SessionEnd(success=false) at ERROR level. `success` comes from the
    // loop outcome above; any error from close() is logged but doesn't
    // mask loop_result.
    if let Err(e) = session.close(success).await {
        tracing::warn!(error = %e, "session close failed after chat loop");
    }

    loop_result
}

/// Inner chat loop — everything between session construction and metrics
/// finalize. Returns `Ok(())` on clean exit (`/exit`, EOF, readline Ctrl+C)
/// and `Err(...)` on any failure; [`run_chat`] always observes this result
/// before calling `finalize` so the TaskRecord reflects reality.
///
/// `provider` / `model` are captured at session construction time and
/// threaded through so every successful send can append to the
/// `known_models.json` cache (Sprint 12 task #72 sub-path 2).
async fn chat_loop(
    agent: &str,
    agent_dir: &std::path::Path,
    provider: &str,
    model: &str,
    session: &mut sage_runtime::SageSession,
    sink: &dyn sage_runtime::event::AgentEventSink,
    cancel_token: &tokio_util::sync::CancellationToken,
) -> anyhow::Result<()> {
    use tokio::io::AsyncBufReadExt as _;

    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());

    println!("Connected to '{agent}'. Type /exit to quit, /reset to clear history.");
    println!("(Ctrl+C to interrupt; EOF / Ctrl+D to quit.)");
    println!();

    loop {
        use std::io::Write as _;
        print!("{agent}> ");
        std::io::stdout().flush()?;

        let mut line = String::new();
        let n = tokio::select! {
            res = stdin.read_line(&mut line) => res?,
            _ = tokio::signal::ctrl_c() => {
                cancel_token.cancel();
                println!();
                println!("^C received; closing chat session.");
                break;
            }
        };
        if n == 0 {
            break; // EOF (Ctrl+D)
        }

        match parse_user_input(&line) {
            ChatInput::Exit => break,
            ChatInput::Reset => {
                session.reset();
                println!("  [session reset]");
            }
            ChatInput::Empty => {}
            ChatInput::Message(text) => {
                send_with_cancel(session, sink, text.trim(), cancel_token).await?;
                // Task #72 sub-path 2: record on each successful send so
                // the known_models cache reflects which (provider, model)
                // the user has actually used. Idempotent — duplicates
                // collapse inside the record_used_model set semantics.
                crate::serve::record_session_model(provider, model);
            }
        }
    }

    // `agent_dir` is retained on the signature for future wiring points
    // (e.g. session archive path, workspace-scoped commands). Suppress
    // the unused-param warning without renaming it since callers pass it
    // positionally.
    let _ = agent_dir;
    Ok(())
}

/// Drive a single `session.send(...)` call, racing it against Ctrl+C.
///
/// Sprint 12 task #69: on Ctrl+C we flip the shared cancel token and await
/// the send to unwind gracefully. The agent loop observes the token at its
/// checkpoints and returns `AgentLoopError::Cancelled`, which we print as a
/// non-fatal message so the chat loop continues to the next readline.
async fn send_with_cancel(
    session: &mut sage_runtime::SageSession,
    sink: &dyn sage_runtime::event::AgentEventSink,
    text: &str,
    cancel_token: &tokio_util::sync::CancellationToken,
) -> anyhow::Result<()> {
    let send_fut = session.send(text, sink);
    tokio::pin!(send_fut);

    let result = tokio::select! {
        res = &mut send_fut => res,
        _ = tokio::signal::ctrl_c() => {
            cancel_token.cancel();
            // Drain the send future so session state unwinds cleanly
            // (cancelled tool_results pushed, AgentEnd emitted).
            let res = (&mut send_fut).await;
            println!();
            println!("^C received; turn cancelled.");
            res
        }
    };

    match result {
        Ok(()) => {
            println!();
            Ok(())
        }
        Err(sage_runtime::SageError::AgentLoop(
            sage_runtime::AgentLoopError::Cancelled,
        )) => {
            // Cancelled is not an error condition at the chat-loop level —
            // the user asked for this. Continue reading the next prompt.
            Ok(())
        }
        Err(e) => Err(anyhow::anyhow!("{e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_user_input: slash commands ─────────────────────────────────────

    #[test]
    fn parse_input_exit_command() {
        assert_eq!(parse_user_input("/exit"), ChatInput::Exit);
    }

    #[test]
    fn parse_input_reset_command() {
        assert_eq!(parse_user_input("/reset"), ChatInput::Reset);
    }

    #[test]
    fn parse_input_exit_case_insensitive() {
        assert_eq!(parse_user_input("/EXIT"), ChatInput::Exit);
        assert_eq!(parse_user_input("/Exit"), ChatInput::Exit);
    }

    #[test]
    fn parse_input_reset_case_insensitive() {
        assert_eq!(parse_user_input("/RESET"), ChatInput::Reset);
        assert_eq!(parse_user_input("/Reset"), ChatInput::Reset);
    }

    // ── parse_user_input: regular messages ───────────────────────────────────

    #[test]
    fn parse_input_regular_message() {
        assert_eq!(
            parse_user_input("hello world"),
            ChatInput::Message("hello world".to_string())
        );
    }

    #[test]
    fn parse_input_unknown_slash_command_is_message_post_task_88() {
        // Task #88: `/<name>` no longer special-cased. Unknown slash input
        // is forwarded to the agent as a regular message; the agent decides
        // whether to treat it as a skill invocation by consulting
        // `workspace/skills/INDEX.md`.
        assert_eq!(
            parse_user_input("/unknown"),
            ChatInput::Message("/unknown".to_string())
        );
    }

    #[test]
    fn parse_input_slash_with_content_is_message_post_task_88() {
        // Task #88: same rationale — `/help me debug this` becomes a plain
        // message, not a skill invocation. No /slash argument extraction.
        assert_eq!(
            parse_user_input("/help me debug this"),
            ChatInput::Message("/help me debug this".to_string())
        );
    }

    #[test]
    fn parse_input_bare_slash_is_message() {
        // "/" alone — no recognized command, treated as a message (not empty, not Exit/Reset)
        assert_eq!(
            parse_user_input("/"),
            ChatInput::Message("/".to_string()),
            "bare '/' should be a Message, not Empty or a command"
        );
    }

    #[test]
    fn parse_input_multi_line_message() {
        let msg = "line1\nline2\nline3";
        assert_eq!(
            parse_user_input(msg),
            ChatInput::Message(msg.to_string())
        );
    }

    #[test]
    fn parse_input_reset_with_args_stays_message() {
        // "/reset now" — built-in commands with trailing text stay as Message
        // so the agent sees the literal user intent rather than silently
        // swallowing the tail.
        assert_eq!(
            parse_user_input("/reset now"),
            ChatInput::Message("/reset now".to_string()),
            "/reset with trailing text should be a Message, not a Reset command"
        );
    }

    #[test]
    fn parse_input_exit_with_args_stays_message() {
        assert_eq!(
            parse_user_input("/exit immediately"),
            ChatInput::Message("/exit immediately".to_string()),
            "/exit with trailing text should be a Message, not an Exit command"
        );
    }

    // ── parse_user_input: whitespace is stripped before command matching ────────

    #[test]
    fn parse_input_exit_with_leading_space_is_exit() {
        // Whitespace is stripped before matching — " /exit" is treated as an Exit command.
        assert_eq!(
            parse_user_input(" /exit"),
            ChatInput::Exit,
            "' /exit' with leading space should be Exit after trimming"
        );
    }

    #[test]
    fn parse_input_exit_with_trailing_space_is_exit() {
        // Whitespace is stripped before matching — "/exit " is treated as an Exit command.
        assert_eq!(
            parse_user_input("/exit "),
            ChatInput::Exit,
            "'/exit ' with trailing space should be Exit after trimming"
        );
    }

    #[test]
    fn parse_input_message_original_preserved_when_not_command() {
        // When input is not a command, the original string (with whitespace) is preserved.
        assert_eq!(
            parse_user_input("  hello world  "),
            ChatInput::Message("  hello world  ".to_string()),
            "non-command input must preserve original whitespace"
        );
    }

    // ── parse_user_input: empty / whitespace ─────────────────────────────────

    #[test]
    fn parse_input_empty_string_is_empty() {
        assert_eq!(parse_user_input(""), ChatInput::Empty);
    }

    #[test]
    fn parse_input_spaces_only_is_empty() {
        assert_eq!(parse_user_input("   "), ChatInput::Empty);
    }

    #[test]
    fn parse_input_tab_only_is_empty() {
        assert_eq!(parse_user_input("\t"), ChatInput::Empty);
    }

    #[test]
    fn parse_input_mixed_whitespace_is_empty() {
        assert_eq!(parse_user_input("  \n  "), ChatInput::Empty);
    }

    #[test]
    fn parse_input_non_empty_message_with_leading_whitespace() {
        assert_eq!(
            parse_user_input("  hello"),
            ChatInput::Message("  hello".to_string()),
            "leading whitespace before text should be a Message with preserved whitespace"
        );
    }

    // ── Task #88: /slash skill invocation removed ─────────────────────────
    //
    // Pre-task-#88 parse_user_input mapped `/<name>` to ChatInput::Skill and
    // loaded a template file. The agent now owns skill discovery (via
    // workspace/skills/INDEX.md + Read tool). We keep a handful of
    // regression tests proving `/<name>` flows through as a plain Message.

    #[test]
    fn parse_input_slash_memory_is_message() {
        assert_eq!(
            parse_user_input("/memory"),
            ChatInput::Message("/memory".to_string())
        );
    }

    #[test]
    fn parse_input_slash_with_hyphen_is_message() {
        assert_eq!(
            parse_user_input("/my-skill"),
            ChatInput::Message("/my-skill".to_string())
        );
    }

    #[test]
    fn parse_input_slash_exit_with_trailing_space_still_exit() {
        // " /exit " — trimmed to "/exit" — must stay Exit.
        assert_eq!(parse_user_input(" /exit "), ChatInput::Exit);
    }

}
