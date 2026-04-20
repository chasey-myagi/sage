//! Application configuration paths and constants.
//!
//! Translated from pi-mono `packages/coding-agent/src/config.ts`.

use std::path::PathBuf;

/// Application name — used in CLI help text and config directory naming.
pub const APP_NAME: &str = "sage";

/// Config directory name under home (e.g. `~/.sage/`).
pub const CONFIG_DIR_NAME: &str = ".sage";

/// Environment variable name for the agent data directory.
pub const ENV_AGENT_DIR: &str = "SAGE_CODING_AGENT_DIR";

/// Application version (from Cargo package).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Expand a path that may start with `~/` to an absolute path.
fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    PathBuf::from(path)
}

/// Return the user's home directory.
fn home_dir() -> PathBuf {
    #[cfg(unix)]
    {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"))
    }
    #[cfg(windows)]
    {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOMEDRIVE").and_then(|d| std::env::var("HOMEPATH").map(|p| d + &p)))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("C:\\Users\\Default"))
    }
}

/// Get the agent config directory (e.g., `~/.sage/agent/`).
///
/// Can be overridden via the `SAGE_CODING_AGENT_DIR` environment variable.
pub fn get_agent_dir() -> PathBuf {
    if let Ok(env_dir) = std::env::var(ENV_AGENT_DIR) {
        return expand_tilde(&env_dir);
    }
    home_dir().join(CONFIG_DIR_NAME).join("agent")
}

/// Get the path to `models.json`.
pub fn get_models_path() -> PathBuf {
    get_agent_dir().join("models.json")
}

/// Get the path to `auth.json`.
pub fn get_auth_path() -> PathBuf {
    get_agent_dir().join("auth.json")
}

/// Get the path to `settings.json`.
pub fn get_settings_path() -> PathBuf {
    get_agent_dir().join("settings.json")
}

/// Get the path to the tools directory.
pub fn get_tools_dir() -> PathBuf {
    get_agent_dir().join("tools")
}

/// Get the path to the managed binaries directory (fd, rg).
pub fn get_bin_dir() -> PathBuf {
    get_agent_dir().join("bin")
}

/// Get the path to prompt templates directory.
pub fn get_prompts_dir() -> PathBuf {
    get_agent_dir().join("prompts")
}

/// Get the path to sessions directory.
pub fn get_sessions_dir() -> PathBuf {
    get_agent_dir().join("sessions")
}

/// Get the path to the debug log file.
pub fn get_debug_log_path() -> PathBuf {
    get_agent_dir().join(format!("{APP_NAME}-debug.log"))
}

/// Get the path to user's custom themes directory.
pub fn get_custom_themes_dir() -> PathBuf {
    get_agent_dir().join("themes")
}

/// Return true if the env flag value represents a truthy boolean
/// (`"1"`, `"true"`, `"yes"` case-insensitive).
pub fn is_truthy_env_flag(value: &str) -> bool {
    value == "1" || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_tilde_home() {
        let result = expand_tilde("~");
        assert!(result.to_str().is_some());
        assert!(!result.to_str().unwrap().contains('~'));
    }

    #[test]
    fn expand_tilde_subpath() {
        let result = expand_tilde("~/foo/bar");
        let s = result.to_str().unwrap();
        assert!(s.ends_with("foo/bar"));
        assert!(!s.starts_with('~'));
    }

    #[test]
    fn expand_tilde_absolute() {
        let result = expand_tilde("/absolute/path");
        assert_eq!(result, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn is_truthy_env_flag_variants() {
        assert!(is_truthy_env_flag("1"));
        assert!(is_truthy_env_flag("true"));
        assert!(is_truthy_env_flag("TRUE"));
        assert!(is_truthy_env_flag("yes"));
        assert!(is_truthy_env_flag("YES"));
        assert!(!is_truthy_env_flag("0"));
        assert!(!is_truthy_env_flag("false"));
        assert!(!is_truthy_env_flag(""));
    }

    #[test]
    fn version_is_nonempty() {
        assert!(!VERSION.is_empty());
    }
}
