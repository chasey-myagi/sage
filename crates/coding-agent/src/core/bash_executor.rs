//! Bash command execution with streaming support and cancellation.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/bash-executor.ts`.
//!
//! Provides a unified bash execution implementation used by the agent session
//! for interactive and RPC modes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

// ============================================================================
// Constants
// ============================================================================

/// Default maximum output size in bytes before truncation (512 KiB).
pub const DEFAULT_MAX_BYTES: usize = 512 * 1024;

/// Maximum rolling in-memory buffer (2× default max).
const MAX_OUTPUT_BYTES: usize = DEFAULT_MAX_BYTES * 2;

// ============================================================================
// Types
// ============================================================================

/// Options for [`execute_bash`].
#[derive(Default)]
pub struct BashExecutorOptions {
    /// Callback for streaming sanitized output chunks.
    pub on_chunk: Option<Box<dyn Fn(&str) + Send + Sync>>,
    /// Cancellation flag; set to `true` to abort the running command.
    pub cancel: Option<Arc<AtomicBool>>,
}

/// Result of a bash command execution.
#[derive(Debug, Clone)]
pub struct BashResult {
    /// Combined stdout + stderr output (sanitized, possibly truncated).
    pub output: String,
    /// Process exit code (`None` if killed/cancelled).
    pub exit_code: Option<i32>,
    /// Whether the command was cancelled.
    pub cancelled: bool,
    /// Whether the output was truncated.
    pub truncated: bool,
    /// Path to temp file containing full output (if output exceeded truncation threshold).
    pub full_output_path: Option<PathBuf>,
}

// ============================================================================
// Helpers
// ============================================================================

/// Strip non-printable / binary bytes, keeping newlines and tabs.
fn sanitize_binary_output(s: &str) -> String {
    s.chars()
        .filter(|&c| c == '\n' || c == '\t' || c == '\r' || !c.is_control())
        .collect()
}

/// Truncate `output` to at most [`DEFAULT_MAX_BYTES`] bytes, keeping the tail.
fn truncate_tail(output: &str) -> (&str, bool) {
    let bytes = output.as_bytes();
    if bytes.len() <= DEFAULT_MAX_BYTES {
        return (output, false);
    }
    // Keep last DEFAULT_MAX_BYTES bytes, aligning to a char boundary.
    let start = bytes.len() - DEFAULT_MAX_BYTES;
    let aligned = (start..bytes.len())
        .find(|&i| output.is_char_boundary(i))
        .unwrap_or(bytes.len());
    (&output[aligned..], true)
}

// ============================================================================
// Main executor
// ============================================================================

/// Execute a bash command in `cwd` with optional streaming and cancellation.
///
/// Mirrors `executeBashWithOperations()` in bash-executor.ts.
pub async fn execute_bash(
    command: &str,
    cwd: &Path,
    options: BashExecutorOptions,
) -> anyhow::Result<BashResult> {
    let cancelled_flag = options
        .cancel
        .clone()
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));

    // Build the child process
    let mut child = Command::new("bash")
        .args(["-c", command])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdout = child.stdout.take().expect("stdout piped");
    let mut stderr = child.stderr.take().expect("stderr piped");

    // Accumulate combined output
    let mut output_chunks: Vec<String> = Vec::new();
    let mut output_bytes: usize = 0;
    let mut total_bytes: usize = 0;

    let mut temp_file: Option<(PathBuf, std::fs::File)> = None;

    // Read stdout and stderr concurrently via select loop
    let mut stdout_buf = vec![0u8; 4096];
    let mut stderr_buf = vec![0u8; 4096];
    let mut stdout_done = false;
    let mut stderr_done = false;

    loop {
        if cancelled_flag.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            break;
        }

        if stdout_done && stderr_done {
            break;
        }

        tokio::select! {
            n = stdout.read(&mut stdout_buf), if !stdout_done => {
                match n {
                    Ok(0) => stdout_done = true,
                    Ok(n) => {
                        let raw = String::from_utf8_lossy(&stdout_buf[..n]);
                        let text = sanitize_binary_output(&raw.replace('\r', ""));
                        process_chunk(&text, &mut total_bytes, &mut output_chunks, &mut output_bytes,
                                      &mut temp_file, &options.on_chunk)?;
                    }
                    Err(_) => stdout_done = true,
                }
            }
            n = stderr.read(&mut stderr_buf), if !stderr_done => {
                match n {
                    Ok(0) => stderr_done = true,
                    Ok(n) => {
                        let raw = String::from_utf8_lossy(&stderr_buf[..n]);
                        let text = sanitize_binary_output(&raw.replace('\r', ""));
                        process_chunk(&text, &mut total_bytes, &mut output_chunks, &mut output_bytes,
                                      &mut temp_file, &options.on_chunk)?;
                    }
                    Err(_) => stderr_done = true,
                }
            }
        }
    }

    // Close temp file if open
    let full_output_path = if let Some((path, mut f)) = temp_file {
        // Flush remaining chunks
        for chunk in &output_chunks {
            let _ = f.write_all(chunk.as_bytes());
        }
        drop(f);
        Some(path)
    } else {
        None
    };

    let is_cancelled = cancelled_flag.load(Ordering::Relaxed);
    let exit_code = if is_cancelled {
        None
    } else {
        match child.wait().await {
            Ok(status) => status.code(),
            Err(_) => None,
        }
    };

    let full_output = output_chunks.join("");
    let (final_output, truncated) = truncate_tail(&full_output);

    Ok(BashResult {
        output: final_output.to_string(),
        exit_code,
        cancelled: is_cancelled,
        truncated,
        full_output_path,
    })
}

/// Process a single chunk of output text: accumulate into rolling buffer, stream to callback,
/// and write to temp file if threshold exceeded.
fn process_chunk(
    text: &str,
    total_bytes: &mut usize,
    output_chunks: &mut Vec<String>,
    output_bytes: &mut usize,
    temp_file: &mut Option<(PathBuf, std::fs::File)>,
    on_chunk: &Option<Box<dyn Fn(&str) + Send + Sync>>,
) -> anyhow::Result<()> {
    *total_bytes += text.len();

    // Create temp file once threshold exceeded
    if *total_bytes > DEFAULT_MAX_BYTES && temp_file.is_none() {
        let path = std::env::temp_dir().join(format!(
            "sage-bash-{}.log",
            ulid::Ulid::new().to_string().to_lowercase()
        ));
        let f = std::fs::File::create(&path)?;
        // Dump existing chunks into the file
        let mut f2 = f;
        for chunk in output_chunks.iter() {
            f2.write_all(chunk.as_bytes())?;
        }
        *temp_file = Some((path, f2));
    }

    // Write to temp file if active
    if let Some((_, f)) = temp_file {
        f.write_all(text.as_bytes())?;
    }

    // Rolling in-memory buffer
    output_chunks.push(text.to_string());
    *output_bytes += text.len();
    while *output_bytes > MAX_OUTPUT_BYTES && output_chunks.len() > 1 {
        let removed = output_chunks.remove(0);
        *output_bytes -= removed.len();
    }

    // Stream callback
    if let Some(cb) = on_chunk {
        cb(text);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn execute_echo_command() {
        let result = execute_bash(
            "echo hello",
            Path::new("/tmp"),
            BashExecutorOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(result.output.trim(), "hello");
        assert_eq!(result.exit_code, Some(0));
        assert!(!result.cancelled);
        assert!(!result.truncated);
    }

    #[tokio::test]
    async fn execute_exit_code() {
        let result = execute_bash("exit 42", Path::new("/tmp"), BashExecutorOptions::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, Some(42));
    }

    #[tokio::test]
    async fn execute_stderr_captured() {
        let result = execute_bash(
            "echo err >&2",
            Path::new("/tmp"),
            BashExecutorOptions::default(),
        )
        .await
        .unwrap();
        assert!(result.output.contains("err"));
    }

    #[tokio::test]
    async fn execute_streaming_callback() {
        let chunks = Arc::new(Mutex::new(Vec::<String>::new()));
        let chunks2 = Arc::clone(&chunks);
        let opts = BashExecutorOptions {
            on_chunk: Some(Box::new(move |s| {
                chunks2.lock().unwrap().push(s.to_string());
            })),
            cancel: None,
        };
        let _ = execute_bash("echo stream_test", Path::new("/tmp"), opts)
            .await
            .unwrap();
        let all = chunks.lock().unwrap().join("");
        assert!(all.contains("stream_test"));
    }

    #[tokio::test]
    async fn execute_cancel() {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel2 = Arc::clone(&cancel);
        let opts = BashExecutorOptions {
            on_chunk: None,
            cancel: Some(cancel),
        };
        // Trigger cancellation asynchronously
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            cancel2.store(true, Ordering::Relaxed);
        });
        let result = execute_bash("sleep 5", Path::new("/tmp"), opts)
            .await
            .unwrap();
        assert!(result.cancelled);
        assert!(result.exit_code.is_none());
    }

    #[test]
    fn sanitize_removes_control_chars() {
        let input = "hello\x00world\x01\nfoo";
        let out = sanitize_binary_output(input);
        assert_eq!(out, "helloworld\nfoo");
    }

    #[test]
    fn truncate_tail_short() {
        let s = "hello";
        let (out, truncated) = truncate_tail(s);
        assert_eq!(out, "hello");
        assert!(!truncated);
    }

    #[test]
    fn truncate_tail_long() {
        // Build a string larger than DEFAULT_MAX_BYTES
        let long = "x".repeat(DEFAULT_MAX_BYTES + 100);
        let (out, truncated) = truncate_tail(&long);
        assert!(truncated);
        assert_eq!(out.len(), DEFAULT_MAX_BYTES);
    }
}
