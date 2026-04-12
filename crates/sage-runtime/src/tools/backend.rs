// ToolBackend — abstracts I/O operations for tools (local vs sandbox).

use std::sync::Arc;

/// A directory entry returned by `list_dir`.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Result of a shell command execution.
#[derive(Debug)]
pub struct ShellOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

/// Backend trait that tools use for I/O operations.
///
/// Two implementations:
/// - `LocalBackend`: executes on the host (current behavior)
/// - `SandboxBackend` (future): delegates to `SandboxHandle` for VM execution
///
/// Environment assumptions:
/// - `shell()` requires `bash` in the execution environment.
/// - `GrepTool` constructs `rg` (ripgrep) commands via `shell()`, so the
///   execution environment must have `rg` installed for grep to work.
#[async_trait::async_trait]
pub trait ToolBackend: Send + Sync {
    /// Execute a shell command, returning combined output and exit status.
    async fn shell(&self, command: &str, timeout_secs: u64) -> Result<ShellOutput, String>;

    /// Read a file's contents.
    async fn read_file(&self, path: &str) -> Result<Vec<u8>, String>;

    /// Write data to a file, creating parent directories as needed.
    async fn write_file(&self, path: &str, data: &[u8]) -> Result<(), String>;

    /// List directory entries.
    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>, String>;
}

/// Local backend — executes directly on the host using tokio I/O.
pub struct LocalBackend;

impl LocalBackend {
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

#[async_trait::async_trait]
impl ToolBackend for LocalBackend {
    async fn shell(&self, command: &str, timeout_secs: u64) -> Result<ShellOutput, String> {
        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c")
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;

        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();

        let stdout_task = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            if let Some(mut pipe) = stdout_pipe {
                let _ = pipe.read_to_end(&mut buf).await;
            }
            buf
        });
        let stderr_task = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            if let Some(mut pipe) = stderr_pipe {
                let _ = pipe.read_to_end(&mut buf).await;
            }
            buf
        });

        match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), child.wait()).await
        {
            Ok(Ok(status)) => {
                let stdout =
                    String::from_utf8_lossy(&stdout_task.await.unwrap_or_default()).into_owned();
                let stderr =
                    String::from_utf8_lossy(&stderr_task.await.unwrap_or_default()).into_owned();
                Ok(ShellOutput {
                    stdout,
                    stderr,
                    success: status.success(),
                })
            }
            Ok(Err(e)) => {
                stdout_task.abort();
                stderr_task.abort();
                Err(format!("process error: {e}"))
            }
            Err(_) => {
                // Timeout — kill the process group
                #[cfg(unix)]
                if let Some(pid) = child.id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                let _ = child.kill().await;
                stdout_task.abort();
                stderr_task.abort();
                Err(format!("command timed out after {timeout_secs}s"))
            }
        }
    }

    async fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        tokio::fs::read(path)
            .await
            .map_err(|e| format!("read {path}: {e}"))
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> Result<(), String> {
        // Create parent directories if needed (unconditional to avoid TOCTOU race)
        if let Some(parent) = std::path::Path::new(path).parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("create dirs for {path}: {e}"))?;
        }
        tokio::fs::write(path, data)
            .await
            .map_err(|e| format!("write {path}: {e}"))
    }

    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>, String> {
        let mut entries = Vec::new();
        let mut dir = tokio::fs::read_dir(path)
            .await
            .map_err(|e| format!("read_dir {path}: {e}"))?;
        while let Some(entry) = dir.next_entry().await.map_err(|e| e.to_string())? {
            let metadata = entry.metadata().await.map_err(|e| e.to_string())?;
            entries.push(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: metadata.is_dir(),
                size: metadata.len(),
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local() -> Arc<dyn ToolBackend> {
        LocalBackend::new()
    }

    fn temp_path(suffix: &str) -> String {
        std::env::temp_dir()
            .join(format!(
                "sage_backend_test_{}_{}",
                std::process::id(),
                suffix
            ))
            .to_string_lossy()
            .into_owned()
    }

    // ===================================================================
    // shell
    // ===================================================================

    #[tokio::test]
    async fn test_local_shell_echo() {
        let backend = local();
        let result = backend.shell("echo hello", 10).await.unwrap();
        assert!(result.success);
        assert_eq!(result.stdout.trim(), "hello");
    }

    #[tokio::test]
    async fn test_local_shell_captures_stderr() {
        let backend = local();
        let result = backend.shell("echo err >&2", 10).await.unwrap();
        assert!(result.success);
        assert_eq!(result.stderr.trim(), "err");
    }

    #[tokio::test]
    async fn test_local_shell_failure_exit_code() {
        let backend = local();
        let result = backend.shell("exit 1", 10).await.unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_local_shell_timeout() {
        let backend = local();
        let result = backend.shell("sleep 60", 1).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("timed out"));
    }

    #[tokio::test]
    async fn test_local_shell_multiline_output() {
        let backend = local();
        let result = backend.shell("echo a; echo b; echo c", 10).await.unwrap();
        assert!(result.success);
        let lines: Vec<&str> = result.stdout.trim().lines().collect();
        assert_eq!(lines, vec!["a", "b", "c"]);
    }

    // ===================================================================
    // read_file
    // ===================================================================

    #[tokio::test]
    async fn test_local_read_file() {
        let backend = local();
        let path = temp_path("read");
        std::fs::write(&path, b"hello world").unwrap();

        let data = backend.read_file(&path).await.unwrap();
        assert_eq!(data, b"hello world");

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_local_read_file_not_found() {
        let backend = local();
        let result = backend.read_file("/nonexistent_12345/file.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_local_read_empty_file() {
        let backend = local();
        let path = temp_path("read_empty");
        std::fs::write(&path, b"").unwrap();

        let data = backend.read_file(&path).await.unwrap();
        assert!(data.is_empty());

        let _ = std::fs::remove_file(&path);
    }

    // ===================================================================
    // write_file
    // ===================================================================

    #[tokio::test]
    async fn test_local_write_file() {
        let backend = local();
        let path = temp_path("write");

        backend.write_file(&path, b"test data").await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"test data");

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_local_write_creates_parent_dirs() {
        let backend = local();
        let dir = temp_path("write_nested");
        let path = format!("{dir}/sub/deep/file.txt");

        backend.write_file(&path, b"nested").await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"nested");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_local_write_overwrites() {
        let backend = local();
        let path = temp_path("write_overwrite");
        std::fs::write(&path, b"old").unwrap();

        backend.write_file(&path, b"new").await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");

        let _ = std::fs::remove_file(&path);
    }

    // ===================================================================
    // list_dir
    // ===================================================================

    #[tokio::test]
    async fn test_local_list_dir() {
        let backend = local();
        let dir = temp_path("listdir");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(format!("{dir}/a.txt"), "a").unwrap();
        std::fs::write(format!("{dir}/b.txt"), "b").unwrap();
        std::fs::create_dir(format!("{dir}/subdir")).unwrap();

        let entries = backend.list_dir(&dir).await.unwrap();
        assert_eq!(entries.len(), 3);
        // Should be sorted by name
        assert_eq!(entries[0].name, "a.txt");
        assert!(!entries[0].is_dir);
        assert_eq!(entries[1].name, "b.txt");
        assert!(!entries[1].is_dir);
        assert_eq!(entries[2].name, "subdir");
        assert!(entries[2].is_dir);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_local_list_dir_not_found() {
        let backend = local();
        let result = backend.list_dir("/nonexistent_dir_12345").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_local_list_dir_empty() {
        let backend = local();
        let dir = temp_path("listdir_empty");
        std::fs::create_dir_all(&dir).unwrap();

        let entries = backend.list_dir(&dir).await.unwrap();
        assert!(entries.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
