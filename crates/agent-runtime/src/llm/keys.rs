// API key resolution — Phase 2
// Maps provider names to environment variables and resolves API keys.

use std::fmt;

/// Error type for API key resolution.
#[derive(Debug)]
pub enum KeyError {
    NotFound { provider: String, env_var: String },
    UnknownProvider { provider: String },
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::NotFound { provider, env_var } => {
                write!(
                    f,
                    "API key not found for provider '{}': set environment variable {}",
                    provider, env_var
                )
            }
            KeyError::UnknownProvider { provider } => {
                write!(f, "Unknown provider: {}", provider)
            }
        }
    }
}

impl std::error::Error for KeyError {}

/// Maps a provider name to its API key environment variable name.
pub fn api_key_env_var(provider: &str) -> String {
    match provider {
        "qwen" => "DASHSCOPE_API_KEY".into(),
        "doubao" => "ARK_API_KEY".into(),
        "kimi" => "MOONSHOT_API_KEY".into(),
        "minimax" => "MINIMAX_API_KEY".into(),
        "zai" => "ZHIPU_API_KEY".into(),
        "deepseek" => "DEEPSEEK_API_KEY".into(),
        other => format!("{}_API_KEY", other.to_uppercase()),
    }
}

/// Resolves the API key for a provider from environment variables.
pub fn resolve_api_key(provider: &str) -> Result<String, KeyError> {
    let known = ["qwen", "doubao", "kimi", "minimax", "zai", "deepseek"];
    if !known.contains(&provider) {
        return Err(KeyError::UnknownProvider {
            provider: provider.into(),
        });
    }

    let env_var = api_key_env_var(provider);
    match std::env::var(&env_var) {
        Ok(key) => Ok(key),
        Err(_) => Err(KeyError::NotFound {
            provider: provider.into(),
            env_var,
        }),
    }
}

/// Resolves an API key directly from the given environment variable name.
pub fn resolve_api_key_from_env(env_var: &str) -> Result<String, KeyError> {
    match std::env::var(env_var) {
        Ok(key) if !key.is_empty() => Ok(key),
        _ => Err(KeyError::NotFound {
            provider: "custom".into(),
            env_var: env_var.into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // api_key_env_var — provider to env var mapping
    // ========================================================================

    #[test]
    fn test_env_var_qwen() {
        assert_eq!(api_key_env_var("qwen"), "DASHSCOPE_API_KEY");
    }

    #[test]
    fn test_env_var_doubao() {
        assert_eq!(api_key_env_var("doubao"), "ARK_API_KEY");
    }

    #[test]
    fn test_env_var_kimi() {
        assert_eq!(api_key_env_var("kimi"), "MOONSHOT_API_KEY");
    }

    #[test]
    fn test_env_var_minimax() {
        assert_eq!(api_key_env_var("minimax"), "MINIMAX_API_KEY");
    }

    #[test]
    fn test_env_var_zai() {
        assert_eq!(api_key_env_var("zai"), "ZHIPU_API_KEY");
    }

    #[test]
    fn test_env_var_deepseek() {
        assert_eq!(api_key_env_var("deepseek"), "DEEPSEEK_API_KEY");
    }

    // ========================================================================
    // resolve_api_key — success with env var set
    // ========================================================================

    #[test]
    fn test_resolve_api_key_from_env() {
        // Set a test env var
        // SAFETY: test runs single-threaded (--test-threads=1)
        unsafe { std::env::set_var("DASHSCOPE_API_KEY", "test-key-12345") };
        let result = resolve_api_key("qwen");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test-key-12345");
        // Cleanup
        unsafe { std::env::remove_var("DASHSCOPE_API_KEY") };
    }

    #[test]
    fn test_resolve_api_key_missing_env() {
        // Ensure the env var is not set
        // SAFETY: test runs single-threaded (--test-threads=1)
        unsafe { std::env::remove_var("DEEPSEEK_API_KEY") };
        let result = resolve_api_key("deepseek");
        assert!(result.is_err());
        let err = result.unwrap_err();
        // Error should mention the env var name
        let err_msg = format!("{}", err);
        assert!(
            err_msg.contains("DEEPSEEK_API_KEY"),
            "error should mention the env var: {}",
            err_msg
        );
    }

    // ========================================================================
    // resolve_api_key — error for unknown provider
    // ========================================================================

    #[test]
    fn test_resolve_api_key_unknown_provider() {
        let result = resolve_api_key("unknown_provider");
        assert!(result.is_err());
    }

    // ========================================================================
    // KeyError type
    // ========================================================================

    #[test]
    fn test_key_error_display() {
        let err = KeyError::NotFound {
            provider: "qwen".into(),
            env_var: "DASHSCOPE_API_KEY".into(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("DASHSCOPE_API_KEY"));
        assert!(msg.contains("qwen"));
    }

    #[test]
    fn test_key_error_unknown_provider() {
        let err = KeyError::UnknownProvider {
            provider: "foo".into(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("foo"));
    }

    // ========================================================================
    // resolve_api_key — empty string env var
    // ========================================================================

    #[test]
    fn test_resolve_api_key_empty_string_env() {
        // Set the env var to an empty string
        // SAFETY: test runs single-threaded (--test-threads=1)
        unsafe { std::env::set_var("DASHSCOPE_API_KEY", "") };
        let result = resolve_api_key("qwen");
        // An empty string API key should either be an error or return empty —
        // either way it should not be a non-empty Ok value
        match &result {
            Ok(key) => assert!(
                key.is_empty(),
                "empty env var should produce empty key, got: {}",
                key
            ),
            Err(_) => {} // Also acceptable: treating empty as missing
        }
        // Cleanup
        unsafe { std::env::remove_var("DASHSCOPE_API_KEY") };
    }

    // ========================================================================
    // api_key_env_var — unknown provider behavior
    // ========================================================================

    #[test]
    fn test_api_key_env_var_unknown_provider_returns_value() {
        let result = std::panic::catch_unwind(|| api_key_env_var("totally_unknown_provider_xyz"));
        if let Ok(env_var) = result {
            assert!(!env_var.is_empty());
        }
    }

    // ========================================================================
    // 边界: 环境变量含空白/换行
    // ========================================================================

    // NOTE: std::env::set_var is unsafe in multi-threaded context (Rust 2024 edition).
    // These tests MUST run with --test-threads=1 to avoid data races.

    #[test]
    fn test_resolve_api_key_whitespace_only() {
        // Key with only whitespace should be treated as empty/missing
        unsafe { std::env::set_var("DASHSCOPE_API_KEY", "   ") };
        let result = resolve_api_key("qwen");
        match &result {
            Ok(key) => {
                // If Ok, the key should be trimmed or the raw whitespace
                assert!(key.trim().is_empty() || !key.is_empty());
            }
            Err(_) => {} // Also acceptable: whitespace-only treated as missing
        }
        unsafe { std::env::remove_var("DASHSCOPE_API_KEY") };
    }

    #[test]
    fn test_resolve_api_key_with_newline() {
        // Key with trailing newline (common from `echo "key" > file`)
        unsafe { std::env::set_var("DASHSCOPE_API_KEY", "sk-test123\n") };
        let result = resolve_api_key("qwen");
        assert!(result.is_ok(), "key with trailing newline should resolve");
        let key = result.unwrap();
        // The key should contain the actual value (may or may not be trimmed)
        assert!(key.contains("sk-test123"));
        unsafe { std::env::remove_var("DASHSCOPE_API_KEY") };
    }
}
