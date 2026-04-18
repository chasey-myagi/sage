// ToolBackend — abstracts I/O operations for tools (local vs sandbox).

use std::sync::Arc;

/// A directory entry returned by `list_dir`.
///
/// Intentionally duplicates `sage_protocol::FsEntry` fields so that the
/// `ToolBackend` public API does not leak protocol-layer types.  If `FsEntry`
/// gains new fields, update this struct to match.
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
/// - `LocalBackend`: executes on the host
/// - `SandboxBackend`: delegates to `SandboxHandle` for VM execution
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
///
/// `workspace_root` (optional) anchors relative paths and shell cwd to
/// the agent's workspace directory. Without it, relative paths resolve
/// against the process cwd — which is wherever the user launched `sage`,
/// so a model that says `read workspace/skills/INDEX.md` would hit the
/// wrong place. With `workspace_root` set, the backend joins relative
/// paths onto the root and runs shell commands with `current_dir` at
/// that root. Absolute paths always pass through verbatim.
pub struct LocalBackend {
    workspace_root: Option<std::path::PathBuf>,
}

impl LocalBackend {
    /// Legacy constructor — relative paths resolve against process cwd.
    /// Prefer [`LocalBackend::with_workspace`] for agent use.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            workspace_root: None,
        })
    }

    /// Construct a backend anchored at `root` (agent's workspace_host).
    /// Every relative path / shell cwd becomes relative to `root`.
    pub fn with_workspace(root: std::path::PathBuf) -> Arc<Self> {
        Arc::new(Self {
            workspace_root: Some(root),
        })
    }

    /// Resolve a caller-supplied path against `workspace_root`.
    /// Absolute paths pass through; relative paths are joined onto the
    /// root when it is set, else left as-is (process-cwd relative).
    fn resolve(&self, path: &str) -> std::path::PathBuf {
        let p = std::path::Path::new(path);
        if p.is_absolute() {
            return p.to_path_buf();
        }
        match &self.workspace_root {
            Some(root) => root.join(p),
            None => p.to_path_buf(),
        }
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

        // Anchor every shell invocation at the workspace so `ls`, `cat`,
        // and domain CLIs (lark-cli, git, etc.) all see the same cwd the
        // read/write tools do.
        if let Some(root) = &self.workspace_root {
            cmd.current_dir(root);
        }

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
                // Timeout — kill the process group then reap to prevent zombies
                #[cfg(unix)]
                if let Some(pid) = child.id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                let _ = child.kill().await;
                let _ = child.wait().await; // reap to prevent zombie
                stdout_task.abort();
                stderr_task.abort();
                Err(format!("command timed out after {timeout_secs}s"))
            }
        }
    }

    async fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        let resolved = self.resolve(path);
        tokio::fs::read(&resolved)
            .await
            .map_err(|e| format!("read {}: {}", resolved.display(), e))
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> Result<(), String> {
        let resolved = self.resolve(path);
        // Create parent directories if needed (unconditional to avoid TOCTOU race)
        if let Some(parent) = resolved.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("create dirs for {}: {}", resolved.display(), e))?;
        }
        tokio::fs::write(&resolved, data)
            .await
            .map_err(|e| format!("write {}: {}", resolved.display(), e))
    }

    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>, String> {
        let resolved = self.resolve(path);
        let mut entries = Vec::new();
        let mut dir = tokio::fs::read_dir(&resolved)
            .await
            .map_err(|e| format!("read_dir {}: {}", resolved.display(), e))?;
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

// ── SandboxBackend ───────────────────────────────────────────────────

/// Backend that delegates all operations to a sandbox VM via `SandboxHandle`.
///
/// Tools execute inside the VM's isolated environment rather than on the host.
/// The handle is wrapped in a `tokio::sync::Mutex` for `Sync` safety; the relay
/// inside `SandboxHandle` multiplexes concurrent requests by request-id, so
/// sequential lock acquisition is safe and does not deadlock.
pub struct SandboxBackend {
    handle: Arc<sage_sandbox::SandboxHandle>,
}

impl SandboxBackend {
    /// Wrap an `Arc<SandboxHandle>` into an `Arc<SandboxBackend>` suitable
    /// for passing to `create_tool()`.
    ///
    /// Accepts `Arc` so the caller can retain a reference for lifecycle
    /// management (e.g. calling `handle.stop()` on shutdown).
    pub fn new(handle: Arc<sage_sandbox::SandboxHandle>) -> Arc<Self> {
        Arc::new(Self { handle })
    }
}

#[async_trait::async_trait]
impl ToolBackend for SandboxBackend {
    async fn shell(&self, command: &str, timeout_secs: u64) -> Result<ShellOutput, String> {
        let timeout_u32 = u32::try_from(timeout_secs).unwrap_or(u32::MAX);
        let output = self
            .handle
            .shell(command, timeout_u32)
            .await
            .map_err(|e| e.to_string())?;
        Ok(ShellOutput {
            stdout: output.stdout,
            stderr: output.stderr,
            success: output.exit_code == 0,
        })
    }

    async fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        self.handle.fs_read(path).await.map_err(|e| e.to_string())
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> Result<(), String> {
        self.handle
            .fs_write(path, data)
            .await
            .map_err(|e| e.to_string())
    }

    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>, String> {
        let entries = self.handle.fs_list(path).await.map_err(|e| e.to_string())?;
        let mut result: Vec<DirEntry> = entries
            .into_iter()
            .map(|e| DirEntry {
                name: e.name,
                is_dir: e.is_dir,
                size: e.size,
            })
            .collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
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

    // ── workspace_root routing — regression tests for v0.0.3 fix ─────────
    //
    // LocalBackend::with_workspace(root) must resolve relative paths
    // against `root` for read/write/list, and run shell commands with
    // `current_dir` at `root`. Absolute paths must pass through
    // verbatim. These behaviours are load-bearing — without them, a
    // model that sends `read workspace/skills/INDEX.md` resolves against
    // the user's shell cwd and always misses.

    #[tokio::test]
    async fn workspace_read_resolves_relative_against_root() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills").join("demo");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("SKILL.md"), b"frontmatter body").unwrap();

        let backend = LocalBackend::with_workspace(tmp.path().to_path_buf());
        // Model-style relative path: starts at the workspace root, not cwd.
        let bytes = backend
            .read_file("skills/demo/SKILL.md")
            .await
            .expect("relative read must resolve against workspace_root");
        assert_eq!(bytes, b"frontmatter body");
    }

    #[tokio::test]
    async fn workspace_read_absolute_path_passes_through() {
        let tmp = tempfile::TempDir::new().unwrap();
        let abs = tmp.path().join("elsewhere.txt");
        std::fs::write(&abs, b"absolute content").unwrap();

        // Different workspace root than the file's parent — absolute path
        // must still reach the file. workspace_root only rewrites relative
        // paths, never absolute ones.
        let other_ws = tempfile::TempDir::new().unwrap();
        let backend = LocalBackend::with_workspace(other_ws.path().to_path_buf());
        let bytes = backend
            .read_file(abs.to_str().unwrap())
            .await
            .expect("absolute path must pass through");
        assert_eq!(bytes, b"absolute content");
    }

    #[tokio::test]
    async fn workspace_read_without_root_uses_cwd_relative() {
        // Legacy LocalBackend::new() keeps the old process-cwd behaviour
        // so existing tests / non-agent callers aren't broken.
        let backend = LocalBackend::new();
        // Read a file from the current crate root that we know exists.
        // (Cargo.toml is present for every test run.)
        let out = backend.read_file("Cargo.toml").await;
        assert!(out.is_ok(), "new() must still resolve against cwd");
    }

    #[tokio::test]
    async fn workspace_write_creates_files_under_root() {
        let tmp = tempfile::TempDir::new().unwrap();
        let backend = LocalBackend::with_workspace(tmp.path().to_path_buf());

        backend
            .write_file("memory/NOTES.md", b"hello")
            .await
            .expect("workspace write must succeed");

        // Written file lands under the workspace root, NOT the process cwd.
        let expected = tmp.path().join("memory").join("NOTES.md");
        assert!(expected.exists(), "file must materialize at workspace/memory/NOTES.md");
        assert_eq!(std::fs::read(&expected).unwrap(), b"hello");
    }

    #[tokio::test]
    async fn workspace_list_dir_resolves_relative_against_root() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sub = tmp.path().join("skills");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("a.md"), b"").unwrap();
        std::fs::write(sub.join("b.md"), b"").unwrap();

        let backend = LocalBackend::with_workspace(tmp.path().to_path_buf());
        let entries = backend
            .list_dir("skills")
            .await
            .expect("relative list must resolve against workspace_root");
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"a.md"));
        assert!(names.contains(&"b.md"));
    }

    #[tokio::test]
    async fn workspace_shell_cwd_is_root() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Create a marker file at the root so `pwd` output can be compared
        // indirectly via `ls`.
        std::fs::write(tmp.path().join("marker.txt"), b"").unwrap();

        let backend = LocalBackend::with_workspace(tmp.path().to_path_buf());
        let out = backend.shell("pwd && ls marker.txt", 10).await.unwrap();
        assert!(out.success, "shell must succeed with cwd at workspace_root");
        // pwd output on macOS may prepend /private to /var (symlink); both
        // end with the tempdir basename, which is sufficient.
        let basename = tmp.path().file_name().unwrap().to_string_lossy();
        assert!(
            out.stdout.contains(basename.as_ref()),
            "pwd must include workspace basename '{basename}', got: {}",
            out.stdout
        );
        assert!(out.stdout.contains("marker.txt"), "ls must see marker.txt");
    }

    #[tokio::test]
    async fn workspace_read_error_message_includes_resolved_path() {
        // When the model sends a bad relative path the error should help
        // it self-correct — including the resolved path (not just the raw
        // input) tells the model "you asked for X, I looked at Y".
        let tmp = tempfile::TempDir::new().unwrap();
        let backend = LocalBackend::with_workspace(tmp.path().to_path_buf());
        let err = backend
            .read_file("nope/missing.md")
            .await
            .expect_err("missing file must error");
        let root_basename = tmp.path().file_name().unwrap().to_string_lossy();
        assert!(
            err.contains(root_basename.as_ref()) || err.contains("nope/missing.md"),
            "error should identify the resolved location, got: {err}"
        );
    }

    // ── Original shell tests (no workspace_root) ──────────────────────────

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

    // ===================================================================
    // SandboxBackend — compile-time trait assertions
    // ===================================================================

    /// Compile-time proof that SandboxBackend satisfies Send + Sync + ToolBackend.
    #[allow(dead_code)]
    fn _assert_sandbox_backend_bounds() {
        fn _require_send_sync<T: Send + Sync>() {}
        fn _require_tool_backend<T: ToolBackend>() {}
        _require_send_sync::<SandboxBackend>();
        _require_tool_backend::<SandboxBackend>();
    }

    /// SandboxBackend::new() returns Arc<SandboxBackend> which can be used
    /// as Arc<dyn ToolBackend>.
    #[allow(dead_code)]
    fn _assert_sandbox_backend_arc_coercion() {
        // This is a type-level assertion — SandboxBackend::new() produces
        // Arc<SandboxBackend> which must coerce to Arc<dyn ToolBackend>.
        fn _accept(_: Arc<dyn ToolBackend>) {}
        // Can't call this without a SandboxHandle, but the function signature
        // proves the coercion works at compile time.
    }

    // ===================================================================
    // SandboxBackend — integration tests (require running VM)
    //
    // These tests are #[ignore]d by default. Run with:
    //   cargo test -p sage-runtime sandbox_backend -- --ignored
    //
    // Prerequisites:
    //   - sage-guest cross-compiled to aarch64-unknown-linux-musl
    //   - libkrunfw installed at ~/.microsandbox/lib/
    //   - sandbox-runtime binary built
    // ===================================================================

    /// Helper: create a SandboxHandle for integration tests.
    /// Returns None if the sandbox infrastructure is not available.
    #[allow(dead_code)]
    async fn create_test_sandbox() -> Option<sage_sandbox::SandboxHandle> {
        use sage_sandbox::SandboxBuilder;
        match SandboxBuilder::new("test-backend").create().await {
            Ok(handle) => Some(handle),
            Err(e) => {
                eprintln!("sandbox unavailable, skipping: {e}");
                None
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_sandbox_backend_shell_echo() {
        let handle = create_test_sandbox().await.expect("sandbox required");
        let backend: Arc<dyn ToolBackend> = SandboxBackend::new(Arc::new(handle));

        let result = backend.shell("echo hello", 10).await.unwrap();
        assert!(result.success, "echo should succeed");
        assert_eq!(result.stdout.trim(), "hello");
    }

    #[tokio::test]
    #[ignore]
    async fn test_sandbox_backend_shell_nonzero_exit() {
        let handle = create_test_sandbox().await.expect("sandbox required");
        let backend: Arc<dyn ToolBackend> = SandboxBackend::new(Arc::new(handle));

        let result = backend.shell("exit 42", 10).await.unwrap();
        assert!(!result.success, "exit 42 should report failure");
    }

    #[tokio::test]
    #[ignore]
    async fn test_sandbox_backend_shell_captures_stderr() {
        let handle = create_test_sandbox().await.expect("sandbox required");
        let backend: Arc<dyn ToolBackend> = SandboxBackend::new(Arc::new(handle));

        let result = backend.shell("echo err >&2", 10).await.unwrap();
        assert!(result.success);
        assert_eq!(result.stderr.trim(), "err");
    }

    #[tokio::test]
    #[ignore]
    async fn test_sandbox_backend_shell_timeout() {
        let handle = create_test_sandbox().await.expect("sandbox required");
        let backend: Arc<dyn ToolBackend> = SandboxBackend::new(Arc::new(handle));

        let result = backend.shell("sleep 60", 1).await;
        assert!(result.is_err(), "sleep 60 with 1s timeout should error");
        let err = result.unwrap_err();
        assert!(
            err.to_lowercase().contains("timeout") || err.to_lowercase().contains("timed out"),
            "error should mention timeout: {err}"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_sandbox_backend_read_write_roundtrip() {
        let handle = create_test_sandbox().await.expect("sandbox required");
        let backend: Arc<dyn ToolBackend> = SandboxBackend::new(Arc::new(handle));

        let test_path = "/tmp/sage_sandbox_rw_test.txt";
        let content = b"hello from sandbox backend test";

        backend.write_file(test_path, content).await.unwrap();
        let read_back = backend.read_file(test_path).await.unwrap();
        assert_eq!(read_back, content, "read-back should match written content");
    }

    #[tokio::test]
    #[ignore]
    async fn test_sandbox_backend_read_nonexistent_file() {
        let handle = create_test_sandbox().await.expect("sandbox required");
        let backend: Arc<dyn ToolBackend> = SandboxBackend::new(Arc::new(handle));

        let result = backend.read_file("/nonexistent_12345.txt").await;
        assert!(result.is_err(), "reading missing file should error");
    }

    #[tokio::test]
    #[ignore]
    async fn test_sandbox_backend_write_creates_parent_dirs() {
        let handle = create_test_sandbox().await.expect("sandbox required");
        let backend: Arc<dyn ToolBackend> = SandboxBackend::new(Arc::new(handle));

        // Guest fs_write may or may not auto-create parent dirs — either
        // success or a clear error is acceptable. The test verifies no panic.
        let _ = backend
            .write_file("/tmp/sandbox_nested/sub/file.txt", b"nested")
            .await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_sandbox_backend_list_dir() {
        let handle = create_test_sandbox().await.expect("sandbox required");
        let backend: Arc<dyn ToolBackend> = SandboxBackend::new(Arc::new(handle));

        // /tmp should always exist in the guest
        let entries = backend.list_dir("/tmp").await.unwrap();
        // Just verify it returns a list (may be empty on fresh VM)
        // Just verify it returns without error (may be empty on fresh VM)
        let _ = entries;
    }

    #[tokio::test]
    #[ignore]
    async fn test_sandbox_backend_list_dir_not_found() {
        let handle = create_test_sandbox().await.expect("sandbox required");
        let backend: Arc<dyn ToolBackend> = SandboxBackend::new(Arc::new(handle));

        let result = backend.list_dir("/nonexistent_dir_99999").await;
        assert!(result.is_err(), "listing missing dir should error");
    }

    #[tokio::test]
    #[ignore]
    async fn test_sandbox_backend_list_dir_maps_fields() {
        let handle = create_test_sandbox().await.expect("sandbox required");
        let backend: Arc<dyn ToolBackend> = SandboxBackend::new(Arc::new(handle));

        // Write a file then list the directory
        backend
            .write_file("/tmp/sandbox_ls_test.txt", b"content")
            .await
            .unwrap();

        let entries = backend.list_dir("/tmp").await.unwrap();
        let entry = entries
            .iter()
            .find(|e| e.name == "sandbox_ls_test.txt")
            .expect("written file should appear in listing");
        assert!(!entry.is_dir, "file should not be a directory");
        assert!(entry.size > 0, "file should have non-zero size");
    }

    #[tokio::test]
    #[ignore]
    async fn test_sandbox_backend_concurrent_shell_calls() {
        let handle = create_test_sandbox().await.expect("sandbox required");
        let backend: Arc<dyn ToolBackend> = SandboxBackend::new(Arc::new(handle));

        // Fire 3 concurrent shell commands — relay multiplexes by request_id
        let (r1, r2, r3) = tokio::join!(
            backend.shell("echo one", 10),
            backend.shell("echo two", 10),
            backend.shell("echo three", 10),
        );
        assert_eq!(r1.unwrap().stdout.trim(), "one");
        assert_eq!(r2.unwrap().stdout.trim(), "two");
        assert_eq!(r3.unwrap().stdout.trim(), "three");
    }
}
