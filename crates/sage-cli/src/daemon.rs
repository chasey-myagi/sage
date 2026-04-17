// Daemon — Unix socket-based background agent server.
//
// Protocol: newline-delimited JSON over a Unix domain socket.
//
// Socket path:  /tmp/sage-<name>.sock
// PID file:     ~/.sage/agents/<name>/daemon.pid

use anyhow::{Context as _, Result};
use sage_runtime::event::{AgentEvent, AgentEventSink};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::Mutex;

// ── Protocol types ───────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Send { text: String },
    Reset,
    Ping,
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    TextDelta { text: String },
    ToolStart { name: String, id: String },
    ToolEnd { id: String, is_error: bool },
    CompactionStart { reason: String, message_count: usize },
    CompactionEnd { tokens_before: u64, messages_compacted: usize },
    RunEnd,
    RunError { error: String },
    Pong,
    ResetOk,
    ShutdownOk,
}

// ── Path helpers ─────────────────────────────────────────────────────

fn socket_path(name: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/sage-{name}.sock"))
}

fn pid_path(name: &str) -> Result<PathBuf> {
    let home = sage_runner::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    Ok(home
        .join(".sage")
        .join("agents")
        .join(name)
        .join("daemon.pid"))
}

fn agents_dir() -> Result<PathBuf> {
    let home = sage_runner::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    Ok(home.join(".sage").join("agents"))
}

async fn read_pid(name: &str) -> Option<u32> {
    let path = pid_path(name).ok()?;
    let s = tokio::fs::read_to_string(&path).await.ok()?;
    s.trim().parse().ok()
}

fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, 0) == 0
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

// ── SocketSink ───────────────────────────────────────────────────────

/// An [`AgentEventSink`] that writes protocol messages as JSON-lines to a socket.
struct SocketSink {
    writer: Arc<Mutex<OwnedWriteHalf>>,
}

#[async_trait::async_trait]
impl AgentEventSink for SocketSink {
    async fn emit(&self, event: AgentEvent) {
        let msg: Option<ServerMsg> = match &event {
            AgentEvent::MessageUpdate { delta, .. } => {
                Some(ServerMsg::TextDelta { text: delta.clone() })
            }
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                ..
            } => Some(ServerMsg::ToolStart {
                name: tool_name.clone(),
                id: tool_call_id.clone(),
            }),
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                is_error,
                ..
            } => Some(ServerMsg::ToolEnd {
                id: tool_call_id.clone(),
                is_error: *is_error,
            }),
            AgentEvent::CompactionStart {
                reason,
                message_count,
            } => Some(ServerMsg::CompactionStart {
                reason: reason.clone(),
                message_count: *message_count,
            }),
            AgentEvent::CompactionEnd {
                tokens_before,
                messages_compacted,
            } => Some(ServerMsg::CompactionEnd {
                tokens_before: *tokens_before,
                messages_compacted: *messages_compacted,
            }),
            AgentEvent::RunError { error } => {
                Some(ServerMsg::RunError { error: error.clone() })
            }
            _ => None,
        };
        if let Some(msg) = msg {
            write_msg_to(&self.writer, &msg).await;
        }
    }
}

async fn write_msg_to(writer: &Arc<Mutex<OwnedWriteHalf>>, msg: &ServerMsg) {
    let Ok(line) = serde_json::to_string(msg) else {
        return;
    };
    let line = line + "\n";
    let mut w = writer.lock().await;
    let _ = w.write_all(line.as_bytes()).await;
}

// ── Daemon server ────────────────────────────────────────────────────

/// Run the daemon server for the named agent.
///
/// Binds a Unix socket, writes a PID file, then accepts one client connection
/// at a time. Runs until a `Shutdown` message is received.
///
/// # Metrics semantics (v0.0.3 #79)
///
/// One `TaskRecord` captures the entire daemon lifetime — not one per client
/// connection. A long-lived daemon that serves five Feishu conversations
/// accumulates turn / tool / token counts into a single record and flushes
/// it to `<workspace>/metrics/<task_id>.json` on shutdown. That matches the
/// way operators think about daemons (one process, one task) and avoids
/// flooding the metrics dir with a file per TCP flap. Finalize fires on
/// both the explicit `Shutdown` exit and any accept-loop error.
///
/// # Known limitation (v0.0.3)
///
/// Hard kills — `SIGKILL`, OOM, process panic / abort — lose any in-flight
/// metrics that haven't been finalized yet. Graceful exits (`Shutdown` msg,
/// accept-loop `Err`) do finalize. Operators needing strict durability
/// should treat the TaskRecord as best-effort observability, not audit
/// log. v0.0.4 may switch to an append-only per-event log to close this.
pub async fn run_server(agent_name: &str, dev: bool) -> Result<()> {
    crate::serve::validate_agent_name(agent_name)?;
    let (config, config_hash) = crate::serve::load_agent_config_with_hash(agent_name).await?;
    let engine = crate::serve::build_engine_for_agent(&config, dev).await?;
    let mut session = engine
        .session()
        .await
        .map_err(|e| anyhow::anyhow!("failed to create session: {e}"))?;

    let sock_path = socket_path(agent_name);
    // Remove stale socket from a previous (crashed) run
    let _ = tokio::fs::remove_file(&sock_path).await;
    let listener = tokio::net::UnixListener::bind(&sock_path)
        .with_context(|| format!("cannot bind socket at {}", sock_path.display()))?;

    let pid = std::process::id();
    let pid_file = pid_path(agent_name)?;
    tokio::fs::write(&pid_file, pid.to_string())
        .await
        .with_context(|| format!("cannot write PID file at {}", pid_file.display()))?;

    tracing::info!(agent = agent_name, pid = pid, socket = ?sock_path, "daemon started");

    // One MetricsCollector for the whole daemon lifetime (see function doc).
    let shared_metrics = build_daemon_metrics(agent_name, &config, config_hash)?;

    let loop_result: Result<()> = async {
        loop {
            let (stream, _addr) = listener
                .accept()
                .await
                .context("failed to accept connection")?;
            let shutdown = handle_client(stream, &mut session, &shared_metrics).await?;
            if shutdown {
                break;
            }
        }
        Ok(())
    }
    .await;

    let _ = tokio::fs::remove_file(&sock_path).await;
    let _ = tokio::fs::remove_file(&pid_file).await;
    finalize_daemon_metrics(&shared_metrics, &loop_result).await;
    tracing::info!(agent = agent_name, "daemon stopped");
    loop_result
}

/// Build the daemon-lifetime [`MetricsCollector`] and wrap it in the
/// shared cell that the per-client `handle_client` call uses to tee
/// events into the record.
///
/// Extracted so unit tests can exercise the config → collector wiring
/// without spinning up the Unix socket listener.
fn build_daemon_metrics(
    agent_name: &str,
    config: &sage_runner::config::AgentConfig,
    config_hash: String,
) -> Result<sage_runner::metrics::SharedMetrics> {
    let agent_dir = crate::serve::sage_agents_dir()?.join(agent_name);
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
    Ok(sage_runner::metrics::share_collector(collector))
}

/// Pull the collector out of the shared cell and call `finalize` with the
/// success / failure derived from the accept-loop outcome.
///
/// Idempotent — double calls are cheap because `take_collector` returns
/// `None` once drained. Designed this way so the callers can always run
/// this on daemon exit without having to track "did we already finalize?"
/// state alongside loop-error handling.
async fn finalize_daemon_metrics(
    shared: &sage_runner::metrics::SharedMetrics,
    loop_result: &Result<()>,
) {
    let Some(collector) = sage_runner::metrics::take_collector(shared).await else {
        return;
    };
    let (success, failure_reason) = match loop_result {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };
    if let Err(e) = collector.finalize(success, failure_reason).await {
        tracing::warn!(error = %e, "metrics finalize failed at daemon stop");
    }
}

/// Handle one client connection. Returns `true` if the client sent `Shutdown`.
///
/// Per-client disconnect (`n == 0`) deliberately does NOT finalize metrics
/// — the next client reconnect continues accumulating into the same
/// daemon-lifetime record. Only the explicit `Shutdown` message terminates
/// the accept loop; finalize then runs once in [`run_server`].
async fn handle_client(
    stream: tokio::net::UnixStream,
    session: &mut sage_runtime::SageSession,
    shared_metrics: &sage_runner::metrics::SharedMetrics,
) -> Result<bool> {
    let (read_half, write_half) = stream.into_split();
    let writer = Arc::new(Mutex::new(write_half));
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .context("failed to read from client")?;
        if n == 0 {
            // Client disconnected cleanly — keep metrics alive for the
            // next reconnect.
            break;
        }

        let msg: ClientMsg = match serde_json::from_str(line.trim()) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(err = %e, raw = line.trim(), "invalid client message — ignored");
                continue;
            }
        };

        match msg {
            ClientMsg::Send { text } => {
                let socket_sink = SocketSink {
                    writer: Arc::clone(&writer),
                };
                let sink = sage_runner::metrics::MetricsSink::new(
                    shared_metrics.clone(),
                    socket_sink,
                );
                match session.send(&text, &sink).await {
                    Ok(()) => write_msg_to(&writer, &ServerMsg::RunEnd).await,
                    Err(e) => {
                        write_msg_to(
                            &writer,
                            &ServerMsg::RunError {
                                error: e.to_string(),
                            },
                        )
                        .await;
                    }
                }
            }
            ClientMsg::Reset => {
                session.reset();
                write_msg_to(&writer, &ServerMsg::ResetOk).await;
            }
            ClientMsg::Ping => {
                write_msg_to(&writer, &ServerMsg::Pong).await;
            }
            ClientMsg::Shutdown => {
                write_msg_to(&writer, &ServerMsg::ShutdownOk).await;
                return Ok(true);
            }
        }
    }

    Ok(false)
}

// ── Client helpers ───────────────────────────────────────────────────

/// Connect to a running daemon and enter an interactive session.
pub async fn connect_interactive(agent_name: &str) -> Result<()> {
    crate::serve::validate_agent_name(agent_name)?;
    let sock_path = socket_path(agent_name);
    let stream = tokio::net::UnixStream::connect(&sock_path)
        .await
        .with_context(|| {
            format!(
                "daemon not running for '{agent_name}' (socket: {})\nHint: run `sage start --agent {agent_name}` first",
                sock_path.display()
            )
        })?;

    let (read_half, write_half) = stream.into_split();
    let writer = Arc::new(Mutex::new(write_half));
    let mut reader = BufReader::new(read_half);
    let mut stdin = BufReader::new(tokio::io::stdin());

    println!("Connected to '{agent_name}'. Type /exit to quit.");
    println!();

    let mut user_line = String::new();
    let mut srv_line = String::new();

    loop {
        use std::io::Write as _;
        print!("{agent_name}> ");
        std::io::stdout().flush()?;

        user_line.clear();
        let n = stdin.read_line(&mut user_line).await?;
        if n == 0 {
            break;
        }
        let text = user_line.trim();
        if text.is_empty() {
            continue;
        }
        if text.eq_ignore_ascii_case("/exit") || text.eq_ignore_ascii_case("/quit") {
            break;
        }
        if text.eq_ignore_ascii_case("/reset") {
            send_msg(&writer, &ClientMsg::Reset).await?;
            // Drain the ResetOk
            srv_line.clear();
            reader.read_line(&mut srv_line).await?;
            println!("  [session reset]");
            continue;
        }

        send_msg(&writer, &ClientMsg::Send { text: text.to_string() }).await?;

        // Stream server responses until RunEnd / RunError
        loop {
            srv_line.clear();
            reader.read_line(&mut srv_line).await?;
            let trimmed = srv_line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let msg: ServerMsg = serde_json::from_str(trimmed)
                .with_context(|| format!("unexpected server message: {trimmed}"))?;
            match msg {
                ServerMsg::TextDelta { text } => {
                    use std::io::Write as _;
                    print!("{text}");
                    std::io::stdout().flush()?;
                }
                ServerMsg::ToolStart { name, id } => {
                    eprintln!("\n  [tool: {name} ({id})]");
                }
                ServerMsg::ToolEnd { is_error, id } => {
                    if is_error {
                        eprintln!("  [tool {id} — ERROR]");
                    }
                }
                ServerMsg::CompactionStart { reason, .. } => {
                    eprintln!("\n  [compacting: {reason}...]");
                }
                ServerMsg::CompactionEnd { messages_compacted, .. } => {
                    eprintln!("  [compacted {messages_compacted} messages]");
                }
                ServerMsg::RunEnd => {
                    println!();
                    break;
                }
                ServerMsg::RunError { error } => {
                    eprintln!("\nError: {error}");
                    break;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Send a single message to a running daemon and print the response.
pub async fn send_one(agent_name: &str, text: &str) -> Result<()> {
    crate::serve::validate_agent_name(agent_name)?;
    let sock_path = socket_path(agent_name);
    let stream = tokio::net::UnixStream::connect(&sock_path)
        .await
        .with_context(|| format!("daemon not running for '{agent_name}'"))?;

    let (read_half, write_half) = stream.into_split();
    let writer = Arc::new(Mutex::new(write_half));
    let mut reader = BufReader::new(read_half);

    send_msg(&writer, &ClientMsg::Send { text: text.to_string() }).await?;

    let mut line = String::new();
    loop {
        line.clear();
        reader.read_line(&mut line).await?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: ServerMsg = serde_json::from_str(trimmed)
            .with_context(|| format!("unexpected server message: {trimmed}"))?;
        match msg {
            ServerMsg::TextDelta { text } => {
                use std::io::Write as _;
                print!("{text}");
                std::io::stdout().flush()?;
            }
            ServerMsg::RunEnd => {
                println!();
                break;
            }
            ServerMsg::RunError { error } => {
                eprintln!("Error: {error}");
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

/// Start the agent daemon as a detached background process.
///
/// Re-execs the current binary with the hidden `__daemon-server__` subcommand.
pub async fn start_daemon(agent_name: &str, dev: bool) -> Result<()> {
    crate::serve::validate_agent_name(agent_name)?;
    // Already running?
    if let Some(pid) = read_pid(agent_name).await {
        if is_process_alive(pid) {
            println!("sage: daemon for '{agent_name}' is already running (PID {pid})");
            return Ok(());
        }
    }

    let exe =
        std::env::current_exe().context("cannot determine path to current executable")?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("__daemon-server__").arg("--agent").arg(agent_name);
    if dev {
        cmd.arg("--dev");
    }
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // Detach from the current session so the child survives when the
        // parent (CLI) exits.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }

    let child = cmd.spawn().context("failed to spawn daemon process")?;
    let child_pid = child.id();

    // Brief pause to allow the daemon to bind the socket before we return
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    println!("sage: started daemon for '{agent_name}' (PID {child_pid})");
    Ok(())
}

/// Gracefully stop a running daemon.
///
/// First tries a `Shutdown` message via the Unix socket; falls back to
/// SIGTERM if the socket is unavailable.
pub async fn stop_daemon(agent_name: &str) -> Result<()> {
    crate::serve::validate_agent_name(agent_name)?;
    let sock_path = socket_path(agent_name);

    if sock_path.exists() {
        if let Ok(stream) = tokio::net::UnixStream::connect(&sock_path).await {
            let (_r, write_half) = stream.into_split();
            let writer = Arc::new(Mutex::new(write_half));
            send_msg(&writer, &ClientMsg::Shutdown).await?;
            println!("sage: stopped daemon for '{agent_name}'");
            return Ok(());
        }
    }

    // Socket not reachable — try PID file fallback
    if let Some(pid) = read_pid(agent_name).await {
        if is_process_alive(pid) {
            #[cfg(unix)]
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
            println!("sage: sent SIGTERM to '{agent_name}' daemon (PID {pid})");
        } else {
            // Stale PID file — clean up
            if let Ok(path) = pid_path(agent_name) {
                let _ = tokio::fs::remove_file(&path).await;
            }
            println!("sage: daemon '{agent_name}' is not running");
        }
    } else {
        println!("sage: daemon '{agent_name}' is not running");
    }

    Ok(())
}

/// Print the status of all registered agent daemons.
pub async fn show_status() -> Result<()> {
    let dir = agents_dir()?;

    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(r) => r,
        Err(_) => {
            println!("No agents registered.");
            return Ok(());
        }
    };

    let mut any = false;
    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_type().await?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let pid_file = entry.path().join("daemon.pid");
        if let Ok(pid_str) = tokio::fs::read_to_string(&pid_file).await {
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                let status = if is_process_alive(pid) {
                    "running"
                } else {
                    "stopped"
                };
                println!("{name:<20} {status:<10} PID {pid}");
                any = true;
            }
        }
    }

    if !any {
        println!("No daemons running.");
    }

    Ok(())
}

// ── Internal helpers ─────────────────────────────────────────────────

async fn send_msg(writer: &Arc<Mutex<OwnedWriteHalf>>, msg: &ClientMsg) -> Result<()> {
    let line = serde_json::to_string(msg)? + "\n";
    let mut w = writer.lock().await;
    w.write_all(line.as_bytes()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! #79 daemon metrics wiring — unit tests on the factored helpers.
    //!
    //! Full daemon integration (socket bind + accept loop + LLM engine)
    //! isn't exercised here; those paths need a live Rune runtime and a
    //! real provider. The lifecycle logic we *do* own is encapsulated in
    //! [`finalize_daemon_metrics`], which is the part most likely to
    //! regress (idempotency, success/failure propagation).
    use super::*;
    use sage_runner::config::SessionType;
    use sage_runner::metrics::{share_collector, MetricsCollector};

    fn new_collector(dir: &std::path::Path) -> MetricsCollector {
        MetricsCollector::new(
            "test-agent".to_string(),
            "kimi".to_string(),
            "moonshot-v1-auto".to_string(),
            SessionType::UserDriven,
            dir.to_path_buf(),
            "sha256:deadbeef".to_string(),
        )
    }

    /// `Ok(())` loop result must produce a success=true TaskRecord on disk.
    #[tokio::test]
    async fn finalize_on_ok_writes_success_record() {
        let dir = tempfile::TempDir::new().unwrap();
        let shared = share_collector(new_collector(dir.path()));
        finalize_daemon_metrics(&shared, &Ok(())).await;
        let metrics = dir.path().join("metrics");
        let mut found = 0;
        for entry in std::fs::read_dir(&metrics).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name == "summary.json" { continue; }
            let json: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(json["success"], true, "success must be true on Ok");
            assert!(json["failure_reason"].is_null());
            found += 1;
        }
        assert_eq!(found, 1, "exactly one per-task record must be written");
    }

    /// `Err(_)` loop result must produce success=false with the error string
    /// preserved as the failure reason so operators can see why the daemon
    /// exited.
    #[tokio::test]
    async fn finalize_on_err_records_failure_reason() {
        let dir = tempfile::TempDir::new().unwrap();
        let shared = share_collector(new_collector(dir.path()));
        let err: Result<()> = Err(anyhow::anyhow!("accept loop exploded"));
        finalize_daemon_metrics(&shared, &err).await;
        let metrics = dir.path().join("metrics");
        let mut found = 0;
        for entry in std::fs::read_dir(&metrics).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name == "summary.json" { continue; }
            let json: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(json["success"], false);
            assert_eq!(json["failure_reason"], "accept loop exploded");
            found += 1;
        }
        assert_eq!(found, 1);
    }

    /// Double-finalize must be a no-op (not panic, not double-write). The
    /// shared cell's `take_collector` returns `None` after the first drain,
    /// so the second call is effectively a read + early return.
    #[tokio::test]
    async fn finalize_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let shared = share_collector(new_collector(dir.path()));
        finalize_daemon_metrics(&shared, &Ok(())).await;
        finalize_daemon_metrics(&shared, &Ok(())).await; // second call: must not panic
        let metrics = dir.path().join("metrics");
        let record_count = std::fs::read_dir(&metrics)
            .unwrap()
            .filter(|e| {
                let name = e.as_ref().unwrap().file_name().into_string().unwrap();
                name != "summary.json"
            })
            .count();
        assert_eq!(record_count, 1, "idempotent — single record, no duplicates");
    }
}
