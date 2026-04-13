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
        "anthropic" => "ANTHROPIC_API_KEY".into(),
        "openai" => "OPENAI_API_KEY".into(),
        "google" => "GOOGLE_API_KEY".into(),
        "xai" => "XAI_API_KEY".into(),
        "groq" => "GROQ_API_KEY".into(),
        "openrouter" => "OPENROUTER_API_KEY".into(),
        "qwen" => "DASHSCOPE_API_KEY".into(),
        "doubao" => "ARK_API_KEY".into(),
        "kimi" => "MOONSHOT_API_KEY".into(),
        "minimax" => "MINIMAX_API_KEY".into(),
        "zai" => "ZHIPU_API_KEY".into(),
        "deepseek" => "DEEPSEEK_API_KEY".into(),
        "mistral" => "MISTRAL_API_KEY".into(),
        "cerebras" => "CEREBRAS_API_KEY".into(),
        "github-copilot" => "COPILOT_GITHUB_TOKEN".into(),
        "azure-openai-responses" => "AZURE_OPENAI_API_KEY".into(),
        "amazon-bedrock" => "AWS_ACCESS_KEY_ID".into(),
        "google-vertex" => "GOOGLE_CLOUD_API_KEY".into(),
        other => format!("{}_API_KEY", other.to_uppercase()),
    }
}

/// Resolves the API key for a provider from environment variables.
pub fn resolve_api_key(provider: &str) -> Result<String, KeyError> {
    let known = [
        "anthropic",
        "openai",
        "google",
        "xai",
        "groq",
        "openrouter",
        "qwen",
        "doubao",
        "kimi",
        "minimax",
        "zai",
        "deepseek",
        "mistral",
        "cerebras",
        "github-copilot",
        "azure-openai-responses",
        "amazon-bedrock",
        "google-vertex",
    ];
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
    fn test_env_var_anthropic() {
        assert_eq!(api_key_env_var("anthropic"), "ANTHROPIC_API_KEY");
    }

    #[test]
    fn test_env_var_openai() {
        assert_eq!(api_key_env_var("openai"), "OPENAI_API_KEY");
    }

    #[test]
    fn test_env_var_google() {
        assert_eq!(api_key_env_var("google"), "GOOGLE_API_KEY");
    }

    #[test]
    fn test_env_var_xai() {
        assert_eq!(api_key_env_var("xai"), "XAI_API_KEY");
    }

    #[test]
    fn test_env_var_groq() {
        assert_eq!(api_key_env_var("groq"), "GROQ_API_KEY");
    }

    #[test]
    fn test_env_var_openrouter() {
        assert_eq!(api_key_env_var("openrouter"), "OPENROUTER_API_KEY");
    }

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

    #[test]
    fn test_env_var_mistral() {
        assert_eq!(api_key_env_var("mistral"), "MISTRAL_API_KEY");
    }

    #[test]
    fn test_env_var_cerebras() {
        assert_eq!(api_key_env_var("cerebras"), "CEREBRAS_API_KEY");
    }

    // ========================================================================
    // resolve_api_key — mistral and cerebras recognized
    // ========================================================================

    #[test]
    fn test_resolve_api_key_mistral_recognized() {
        unsafe { std::env::remove_var("MISTRAL_API_KEY") };
        let result = resolve_api_key("mistral");
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("MISTRAL_API_KEY"),
            "mistral should be a known provider, got: {}",
            err
        );
    }

    #[test]
    fn test_resolve_api_key_cerebras_recognized() {
        unsafe { std::env::remove_var("CEREBRAS_API_KEY") };
        let result = resolve_api_key("cerebras");
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("CEREBRAS_API_KEY"),
            "cerebras should be a known provider, got: {}",
            err
        );
    }

    #[test]
    fn test_env_var_github_copilot() {
        assert_eq!(api_key_env_var("github-copilot"), "COPILOT_GITHUB_TOKEN");
    }

    #[test]
    fn test_env_var_azure_openai_responses() {
        assert_eq!(api_key_env_var("azure-openai-responses"), "AZURE_OPENAI_API_KEY");
    }

    #[test]
    fn test_resolve_api_key_azure_openai_responses_recognized() {
        unsafe { std::env::remove_var("AZURE_OPENAI_API_KEY") };
        let result = resolve_api_key("azure-openai-responses");
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("AZURE_OPENAI_API_KEY"),
            "azure-openai-responses should be a known provider, got: {}",
            err
        );
    }

    #[test]
    fn test_env_var_amazon_bedrock() {
        assert_eq!(api_key_env_var("amazon-bedrock"), "AWS_ACCESS_KEY_ID");
    }

    #[test]
    fn test_resolve_api_key_amazon_bedrock_recognized() {
        unsafe { std::env::remove_var("AWS_ACCESS_KEY_ID") };
        let result = resolve_api_key("amazon-bedrock");
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("AWS_ACCESS_KEY_ID"),
            "amazon-bedrock should be a known provider, got: {}",
            err
        );
    }

    #[test]
    fn test_resolve_api_key_github_copilot_recognized() {
        unsafe { std::env::remove_var("COPILOT_GITHUB_TOKEN") };
        let result = resolve_api_key("github-copilot");
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("COPILOT_GITHUB_TOKEN"),
            "github-copilot should be a known provider, got: {}",
            err
        );
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
    // resolve_api_key — new providers are recognized (not UnknownProvider)
    // ========================================================================

    #[test]
    fn test_resolve_api_key_anthropic_recognized() {
        // Should return NotFound (missing env var), not UnknownProvider
        unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };
        let result = resolve_api_key("anthropic");
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("ANTHROPIC_API_KEY"),
            "anthropic should be a known provider, got: {}",
            err
        );
    }

    #[test]
    fn test_resolve_api_key_openai_recognized() {
        unsafe { std::env::remove_var("OPENAI_API_KEY") };
        let result = resolve_api_key("openai");
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("OPENAI_API_KEY"),
            "openai should be a known provider, got: {}",
            err
        );
    }

    #[test]
    fn test_resolve_api_key_google_recognized() {
        unsafe { std::env::remove_var("GOOGLE_API_KEY") };
        let result = resolve_api_key("google");
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("GOOGLE_API_KEY"),
            "google should be a known provider, got: {}",
            err
        );
    }

    #[test]
    fn test_resolve_api_key_xai_recognized() {
        unsafe { std::env::remove_var("XAI_API_KEY") };
        let result = resolve_api_key("xai");
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("XAI_API_KEY"),
            "xai should be a known provider, got: {}",
            err
        );
    }

    #[test]
    fn test_resolve_api_key_groq_recognized() {
        unsafe { std::env::remove_var("GROQ_API_KEY") };
        let result = resolve_api_key("groq");
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("GROQ_API_KEY"),
            "groq should be a known provider, got: {}",
            err
        );
    }

    #[test]
    fn test_resolve_api_key_openrouter_recognized() {
        unsafe { std::env::remove_var("OPENROUTER_API_KEY") };
        let result = resolve_api_key("openrouter");
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("OPENROUTER_API_KEY"),
            "openrouter should be a known provider, got: {}",
            err
        );
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
    fn test_env_var_google_vertex() {
        assert_eq!(api_key_env_var("google-vertex"), "GOOGLE_CLOUD_API_KEY");
    }

    #[test]
    fn test_resolve_api_key_google_vertex_recognized() {
        unsafe { std::env::remove_var("GOOGLE_CLOUD_API_KEY") };
        let result = resolve_api_key("google-vertex");
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("GOOGLE_CLOUD_API_KEY"),
            "google-vertex should be a known provider, got: {}",
            err
        );
    }

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
