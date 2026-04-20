//! Child process utilities.
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/child-process.ts`.

use std::time::Duration;

use tokio::process::Child;

const EXIT_STDIO_GRACE_MS: u64 = 100;

/// Wait for a child process to finish.
///
/// Equivalent to the TypeScript `waitForChildProcess` — waits for the child
/// to exit and for its stdio pipes to close, with a short grace period so
/// that inherited-handle situations on Windows don't hang forever.
///
/// Returns the exit code, or `None` if the process was killed by a signal.
pub async fn wait_for_child_process(mut child: Child) -> anyhow::Result<Option<i32>> {
    // On Unix this is straightforward — `wait()` already returns once the
    // process has exited and all its stdio is closed.  The grace-period logic
    // from the TypeScript version is only relevant on Windows where daemonised
    // descendants can keep the pipe handles alive; we replicate the structure
    // here for parity but on Unix it resolves immediately after exit.
    let status = tokio::select! {
        res = child.wait() => res?,
        _ = tokio::time::sleep(Duration::from_millis(EXIT_STDIO_GRACE_MS)) => {
            // Briefly wait for stdio then force-close.
            let _ = child.kill().await;
            child.wait().await?
        }
    };
    Ok(status.code())
}

/// Run a command, capturing stdout, with an optional timeout.
pub async fn run_command_capture(
    command: &str,
    args: &[&str],
    cwd: Option<&std::path::Path>,
    timeout_ms: Option<u64>,
) -> anyhow::Result<String> {
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    let child = cmd.spawn()?;

    let output = if let Some(ms) = timeout_ms {
        tokio::select! {
            res = child.wait_with_output() => res?,
            _ = tokio::time::sleep(Duration::from_millis(ms)) => {
                return Err(anyhow::anyhow!("{} {:?} timed out after {}ms", command, args, ms));
            }
        }
    } else {
        child.wait_with_output().await?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(anyhow::anyhow!(
            "{} {:?} failed with code {:?}: {}",
            command,
            args,
            output.status.code(),
            if stderr.is_empty() { stdout } else { stderr }
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Run a command (inheriting stdio), returning an error on non-zero exit.
pub async fn run_command(
    command: &str,
    args: &[&str],
    cwd: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let status = cmd.status().await?;
    if !status.success() {
        return Err(anyhow::anyhow!(
            "{} {:?} failed with code {:?}",
            command,
            args,
            status.code()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_echo_capture() {
        let out = run_command_capture("echo", &["hello"], None, None)
            .await
            .unwrap();
        assert_eq!(out, "hello");
    }

    #[tokio::test]
    async fn run_command_failure() {
        let result = run_command("false", &[], None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn timeout_respected() {
        let result = run_command_capture("sleep", &["10"], None, Some(50)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }
}
