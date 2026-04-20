//! Resolve configuration values that may be shell commands, env vars, or literals.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/resolve-config-value.ts`.
//!
//! Used by `auth_storage` and `model_registry`.

use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;

// ============================================================================
// Command result cache
// ============================================================================

static COMMAND_CACHE: std::sync::OnceLock<Mutex<HashMap<String, Option<String>>>> =
    std::sync::OnceLock::new();

fn command_cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    COMMAND_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

// ============================================================================
// resolve_config_value
// ============================================================================

/// Resolve a config value (API key, header value, etc.) to an actual string.
///
/// - If value starts with `!`, executes the rest as a shell command and returns
///   stdout (result is cached for the process lifetime).
/// - Otherwise checks if the value names an env var. If the env var is set,
///   returns its value; otherwise returns the literal value.
///
/// Returns `None` if a shell command fails or produces empty output.
pub async fn resolve_config_value(config: &str) -> Option<String> {
    if config.starts_with('!') {
        execute_command(config)
    } else {
        let env_value = std::env::var(config).ok().filter(|s| !s.is_empty());
        Some(env_value.unwrap_or_else(|| config.to_string()))
    }
}

/// Execute a shell command (the `!cmd` form) and return trimmed stdout.
///
/// Results are cached by the full config string (including the `!` prefix).
fn execute_command(command_config: &str) -> Option<String> {
    {
        let cache = command_cache().lock().expect("command cache lock");
        if let Some(cached) = cache.get(command_config) {
            return cached.clone();
        }
    }

    let command = &command_config[1..]; // strip leading '!'
    let result = run_shell_command(command);

    command_cache()
        .lock()
        .expect("command cache lock")
        .insert(command_config.to_string(), result.clone());

    result
}

/// Run `command` via the system shell and return trimmed stdout, or None on failure.
fn run_shell_command(command: &str) -> Option<String> {
    #[cfg(windows)]
    {
        let output = Command::new("cmd")
            .args(["/C", command])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() { None } else { Some(stdout) }
    }

    #[cfg(not(windows))]
    {
        let output = Command::new("sh")
            .args(["-c", command])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() { None } else { Some(stdout) }
    }
}

// ============================================================================
// resolve_headers
// ============================================================================

/// Resolve all values in a header map using the same resolution logic as API keys.
pub async fn resolve_headers(
    headers: Option<&HashMap<String, String>>,
) -> Option<HashMap<String, String>> {
    let headers = headers?;
    let mut resolved: HashMap<String, String> = HashMap::new();

    for (key, value) in headers {
        if let Some(resolved_value) = resolve_config_value(value).await {
            resolved.insert(key.clone(), resolved_value);
        }
    }

    if resolved.is_empty() { None } else { Some(resolved) }
}

// ============================================================================
// clear_config_value_cache  (for testing)
// ============================================================================

/// Clear the shell-command result cache. Exported for testing.
pub fn clear_config_value_cache() {
    if let Some(cache) = COMMAND_CACHE.get() {
        cache.lock().expect("command cache lock").clear();
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_value_returned_as_is() {
        let result = futures::executor::block_on(resolve_config_value("hello-world"));
        assert_eq!(result, Some("hello-world".to_string()));
    }

    #[test]
    fn env_var_name_resolved_from_env() {
        // SAFETY: test-only, single-threaded context
        unsafe { std::env::set_var("RESOLVE_TEST_VAR_UNIQUE", "my-api-key") };
        let result =
            futures::executor::block_on(resolve_config_value("RESOLVE_TEST_VAR_UNIQUE"));
        assert_eq!(result, Some("my-api-key".to_string()));
        // SAFETY: test-only
        unsafe { std::env::remove_var("RESOLVE_TEST_VAR_UNIQUE") };
    }

    #[test]
    fn unknown_env_var_returns_literal() {
        // Ensure not set
        // SAFETY: test-only
        unsafe { std::env::remove_var("TOTALLY_UNKNOWN_VAR_XYZ") };
        let result =
            futures::executor::block_on(resolve_config_value("TOTALLY_UNKNOWN_VAR_XYZ"));
        assert_eq!(result, Some("TOTALLY_UNKNOWN_VAR_XYZ".to_string()));
    }

    #[cfg(not(windows))]
    #[test]
    fn bang_prefix_executes_shell_command() {
        clear_config_value_cache();
        let result = futures::executor::block_on(resolve_config_value("!echo hello"));
        assert_eq!(result, Some("hello".to_string()));
        clear_config_value_cache();
    }

    #[cfg(not(windows))]
    #[test]
    fn bang_prefix_failing_command_returns_none() {
        clear_config_value_cache();
        let result =
            futures::executor::block_on(resolve_config_value("!exit 1"));
        assert_eq!(result, None);
        clear_config_value_cache();
    }
}
