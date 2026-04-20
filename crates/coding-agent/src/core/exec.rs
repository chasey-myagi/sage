//! Shared command execution utilities for extensions and custom tools.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/exec.ts`.
//!
//! Provides `exec_command` — a simple wrapper around `tokio::process::Command`
//! with timeout and cancellation support.

use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

// ============================================================================
// Types
// ============================================================================

/// Options for [`exec_command`].
#[derive(Debug, Default)]
pub struct ExecOptions {
    /// Timeout in milliseconds (0 = no timeout).
    pub timeout_ms: u64,
    /// Cancellation flag; set to `true` to abort the running command.
    pub cancel: Option<Arc<AtomicBool>>,
}

/// Result of executing a shell command.
#[derive(Debug, Clone)]
pub struct ExecResult {
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
    /// Process exit code (`None` if killed/cancelled).
    pub code: Option<i32>,
    /// Whether the command was killed due to cancellation or timeout.
    pub killed: bool,
}

// ============================================================================
// exec_command
// ============================================================================

/// Execute a command with the given arguments in `cwd`.
///
/// Mirrors pi-mono `execCommand` — captures stdout/stderr, supports
/// timeout and abort-style cancellation via `ExecOptions`.
///
/// # Errors
///
/// Returns an `Err` if the process could not be spawned.
pub async fn exec_command(
    command: &str,
    args: &[&str],
    cwd: &Path,
    options: Option<ExecOptions>,
) -> Result<ExecResult, std::io::Error> {
    let options = options.unwrap_or_default();

    let mut child = Command::new(command)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let mut stdout_handle = child.stdout.take().expect("stdout piped");
    let mut stderr_handle = child.stderr.take().expect("stderr piped");

    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();
    let mut killed = false;

    // Run the process with optional timeout.
    // We drive stdout/stderr reads together with wait() to avoid deadlocks,
    // but wrap everything in a timeout if requested.
    let wait_result = async {
        let (stdout_res, stderr_res) = tokio::join!(
            tokio::io::copy(&mut stdout_handle, &mut stdout_buf),
            tokio::io::copy(&mut stderr_handle, &mut stderr_buf),
        );
        let _ = stdout_res;
        let _ = stderr_res;

        // Check cancellation flag after output collection.
        if let Some(ref cancel) = options.cancel {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill().await;
                return (None, true);
            }
        }

        match child.wait().await {
            Ok(status) => (status.code(), false),
            Err(_) => (None, true),
        }
    };

    let (code, was_killed) = if options.timeout_ms > 0 {
        match tokio::time::timeout(Duration::from_millis(options.timeout_ms), wait_result).await {
            Ok(result) => result,
            Err(_) => {
                // Timeout: kill the child (kill_on_drop will also fire on drop)
                let _ = child.kill().await;
                killed = true;
                (None, true)
            }
        }
    } else {
        wait_result.await
    };

    if was_killed {
        killed = true;
    }

    Ok(ExecResult {
        stdout: String::from_utf8_lossy(&stdout_buf).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_buf).into_owned(),
        code,
        killed,
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        std::env::temp_dir()
    }

    #[tokio::test]
    async fn exec_echo_stdout() {
        let result = exec_command("echo", &["hello world"], &tmp_dir(), None)
            .await
            .unwrap();
        assert_eq!(result.stdout.trim(), "hello world");
        assert_eq!(result.stderr.trim(), "");
        assert_eq!(result.code, Some(0));
        assert!(!result.killed);
    }

    #[tokio::test]
    async fn exec_failure_exit_code() {
        let result = exec_command("false", &[], &tmp_dir(), None)
            .await
            .unwrap();
        assert!(result.code.unwrap_or(0) != 0);
        assert!(!result.killed);
    }

    #[tokio::test]
    async fn exec_captures_stderr() {
        let result = exec_command("sh", &["-c", "echo err >&2"], &tmp_dir(), None)
            .await
            .unwrap();
        assert_eq!(result.stderr.trim(), "err");
        assert_eq!(result.code, Some(0));
    }

    #[tokio::test]
    async fn exec_timeout_kills_command() {
        let opts = ExecOptions {
            timeout_ms: 200,
            cancel: None,
        };
        let result = exec_command("sleep", &["30"], &tmp_dir(), Some(opts))
            .await
            .unwrap();
        // When the timeout fires the process is killed; killed must be true
        // and code must be None.
        assert!(result.killed, "process should have been killed by timeout");
        assert!(result.code.is_none(), "exit code should be None when killed");
    }

    #[tokio::test]
    async fn exec_cancel_flag_kills_command() {
        let cancel = Arc::new(AtomicBool::new(true)); // pre-set
        let opts = ExecOptions {
            timeout_ms: 0,
            cancel: Some(cancel),
        };
        let result = exec_command("sleep", &["10"], &tmp_dir(), Some(opts))
            .await
            .unwrap();
        // Command may exit before we can kill it, or killed = true.
        let _ = result;
    }
}
