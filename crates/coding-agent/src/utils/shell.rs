//! Shell discovery and output sanitisation helpers.
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/shell.ts`.
//!
//! Resolution order for the bash executable:
//! 1. User-specified `shellPath` in settings (validated to exist).
//! 2. On Windows: Git Bash in `%ProgramFiles%\Git\bin\bash.exe`, then
//!    `%ProgramFiles(x86)%\Git\bin\bash.exe`, then `where bash.exe`.
//! 3. On Unix: `/bin/bash`, then `which bash`, else fall back to `sh`.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::config::{get_bin_dir, get_settings_path};
use crate::core::settings_manager::SettingsManager;

/// Cached shell configuration. Cleared by tests.
static SHELL_CONFIG_CACHE: OnceLock<Mutex<Option<ShellConfig>>> = OnceLock::new();

fn cache() -> &'static Mutex<Option<ShellConfig>> {
    SHELL_CONFIG_CACHE.get_or_init(|| Mutex::new(None))
}

/// Configuration for spawning a subprocess via a shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellConfig {
    pub shell: PathBuf,
    pub args: Vec<String>,
}

impl ShellConfig {
    fn new(shell: impl Into<PathBuf>, args: &[&str]) -> Self {
        Self {
            shell: shell.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// Reset the cached shell config. Intended for tests.
pub fn reset_shell_config_cache() {
    if let Ok(mut guard) = cache().lock() {
        *guard = None;
    }
}

/// Locate `bash` on PATH, returning the first match if it exists.
fn find_bash_on_path() -> Option<PathBuf> {
    let (tool, arg) = if cfg!(windows) {
        ("where", "bash.exe")
    } else {
        ("which", "bash")
    };

    let output = std::process::Command::new(tool).arg(arg).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let first = stdout.lines().next()?.trim().to_string();
    if first.is_empty() {
        return None;
    }
    let candidate = PathBuf::from(first);

    // On Windows `where` can return non-existent paths; double-check.
    if cfg!(windows) && !candidate.exists() {
        return None;
    }
    Some(candidate)
}

/// Resolve the shell configuration (cached).
pub fn get_shell_config() -> Result<ShellConfig, ShellError> {
    if let Ok(guard) = cache().lock()
        && let Some(cfg) = guard.as_ref()
    {
        return Ok(cfg.clone());
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let agent_dir = crate::config::get_agent_dir();
    let settings = SettingsManager::create(cwd, agent_dir);

    let config = resolve_shell_config(settings.get_shell_path())?;

    if let Ok(mut guard) = cache().lock() {
        *guard = Some(config.clone());
    }
    Ok(config)
}

fn resolve_shell_config(custom_shell_path: Option<&str>) -> Result<ShellConfig, ShellError> {
    // 1. User-specified shell path.
    if let Some(custom) = custom_shell_path {
        let p = PathBuf::from(custom);
        if p.exists() {
            return Ok(ShellConfig::new(p, &["-c"]));
        }
        return Err(ShellError::CustomShellNotFound {
            path: custom.to_string(),
            settings_path: get_settings_path(),
        });
    }

    if cfg!(windows) {
        // 2a. Git for Windows known locations.
        let mut candidates = Vec::new();
        if let Ok(p) = std::env::var("ProgramFiles") {
            candidates.push(format!(r"{p}\Git\bin\bash.exe"));
        }
        if let Ok(p) = std::env::var("ProgramFiles(x86)") {
            candidates.push(format!(r"{p}\Git\bin\bash.exe"));
        }

        for cand in &candidates {
            let path = Path::new(cand);
            if path.exists() {
                return Ok(ShellConfig::new(path, &["-c"]));
            }
        }

        // 2b. bash on PATH (Cygwin, MSYS2, WSL, etc.)
        if let Some(bash) = find_bash_on_path() {
            return Ok(ShellConfig::new(bash, &["-c"]));
        }

        return Err(ShellError::NoShellFound {
            searched: candidates,
            settings_path: get_settings_path(),
        });
    }

    // 3. Unix: /bin/bash, which bash, else sh.
    let bin_bash = Path::new("/bin/bash");
    if bin_bash.exists() {
        return Ok(ShellConfig::new(bin_bash, &["-c"]));
    }
    if let Some(bash) = find_bash_on_path() {
        return Ok(ShellConfig::new(bash, &["-c"]));
    }
    Ok(ShellConfig::new("sh", &["-c"]))
}

/// Errors produced while locating a shell.
#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    #[error("Custom shell path not found: {path}\nPlease update shellPath in {}", .settings_path.display())]
    CustomShellNotFound {
        path: String,
        settings_path: PathBuf,
    },
    #[error(
        "No bash shell found. Options:\n  1. Install Git for Windows: https://git-scm.com/download/win\n  2. Add your bash to PATH (Cygwin, MSYS2, etc.)\n  3. Set shellPath in {}\n\nSearched Git Bash in:\n  {}",
        .settings_path.display(),
        .searched.join("\n  ")
    )]
    NoShellFound {
        searched: Vec<String>,
        settings_path: PathBuf,
    },
}

/// Return a copy of the current environment with the managed `bin/` directory
/// prepended to `PATH`.
pub fn get_shell_env() -> Vec<(String, String)> {
    let bin_dir = get_bin_dir();
    let bin_dir_str = bin_dir.to_string_lossy().to_string();

    let path_key = std::env::vars()
        .map(|(k, _)| k)
        .find(|k| k.eq_ignore_ascii_case("PATH"))
        .unwrap_or_else(|| "PATH".to_string());

    let delim = if cfg!(windows) { ';' } else { ':' };
    let current_path = std::env::var(&path_key).unwrap_or_default();

    let has_bin_dir = current_path.split(delim).any(|entry| entry == bin_dir_str);

    let updated_path = if has_bin_dir {
        current_path.clone()
    } else if current_path.is_empty() {
        bin_dir_str.clone()
    } else {
        format!("{bin_dir_str}{delim}{current_path}")
    };

    let mut env: Vec<(String, String)> = std::env::vars().collect();
    // Replace existing PATH entry (case-insensitive on Windows, exact on Unix).
    env.retain(|(k, _)| !k.eq_ignore_ascii_case(&path_key));
    env.push((path_key, updated_path));
    env
}

/// Sanitize binary output for display or storage.
///
/// Matches pi-mono `sanitizeBinaryOutput`: keeps tab/newline/CR, filters
/// other control characters, Unicode format characters (U+FFF9..U+FFFB),
/// and invalid code points.
pub fn sanitize_binary_output(input: &str) -> String {
    input
        .chars()
        .filter(|c| {
            let code = *c as u32;
            // Allow tab, newline, carriage return.
            if code == 0x09 || code == 0x0a || code == 0x0d {
                return true;
            }
            // Filter control chars (0x00..=0x1f).
            if code <= 0x1f {
                return false;
            }
            // Filter Unicode format characters that crash string-width.
            if (0xfff9..=0xfffb).contains(&code) {
                return false;
            }
            true
        })
        .collect()
}

/// Kill a process (and its children, on Unix via process-group kill).
#[cfg(unix)]
pub fn kill_process_tree(pid: i32) {
    // SAFETY: libc::kill is safe to call with any pid/sig; errors are ignored.
    unsafe {
        if libc::kill(-pid, libc::SIGKILL) != 0 {
            let _ = libc::kill(pid, libc::SIGKILL);
        }
    }
}

#[cfg(windows)]
pub fn kill_process_tree(pid: i32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_tab_newline_cr() {
        let out = sanitize_binary_output("a\tb\nc\rd");
        assert_eq!(out, "a\tb\nc\rd");
    }

    #[test]
    fn sanitize_strips_low_control_chars() {
        let input = "hello\x01\x02world";
        assert_eq!(sanitize_binary_output(input), "helloworld");
    }

    #[test]
    fn sanitize_strips_unicode_format_chars() {
        let input = "a\u{fff9}b\u{fffb}c";
        assert_eq!(sanitize_binary_output(input), "abc");
    }

    #[test]
    fn sanitize_keeps_normal_text() {
        let s = "Hello, world! 中文 ✓";
        assert_eq!(sanitize_binary_output(s), s);
    }

    #[test]
    fn resolve_shell_config_unix_fallback() {
        if cfg!(unix) {
            let cfg = resolve_shell_config(None).expect("unix should always resolve");
            let shell = cfg.shell.to_string_lossy();
            assert!(
                shell == "/bin/bash" || shell.ends_with("bash") || shell == "sh",
                "unexpected shell: {shell}"
            );
            assert_eq!(cfg.args, vec!["-c".to_string()]);
        }
    }

    #[test]
    fn resolve_shell_config_custom_valid() {
        // Use `/bin/sh` which exists on every Unix.
        if cfg!(unix) {
            let cfg = resolve_shell_config(Some("/bin/sh")).unwrap();
            assert_eq!(cfg.shell, PathBuf::from("/bin/sh"));
        }
    }

    #[test]
    fn resolve_shell_config_custom_missing_errors() {
        let err = resolve_shell_config(Some("/does/not/exist/xyz/bash")).unwrap_err();
        assert!(matches!(err, ShellError::CustomShellNotFound { .. }));
    }

    #[test]
    fn shell_env_contains_bin_dir_in_path() {
        let env = get_shell_env();
        let path = env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let bin_dir = get_bin_dir().to_string_lossy().to_string();
        assert!(path.contains(&bin_dir), "PATH missing bin dir: {path}");
    }
}
