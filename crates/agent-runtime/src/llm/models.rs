// Built-in model catalog — Phase 2
// Registry of known LLM models and their configurations.

use std::sync::LazyLock;

use super::types::*;

static MODEL_CATALOG: LazyLock<Vec<Model>> = LazyLock::new(|| {
    vec![
        // ── Qwen ──
        Model {
            id: "qwen-plus".into(),
            provider: "qwen".into(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            api_key_env: "DASHSCOPE_API_KEY".into(),
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.8,
                output_per_million: 2.0,
                cache_read_per_million: 0.2,
                cache_write_per_million: 0.0,
            },
            compat: default_compat(),
        },
        Model {
            id: "qwen-max".into(),
            provider: "qwen".into(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            api_key_env: "DASHSCOPE_API_KEY".into(),
            max_tokens: 8192,
            context_window: 32768,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 6.0,
                cache_read_per_million: 0.5,
                cache_write_per_million: 0.0,
            },
            compat: default_compat(),
        },
        Model {
            id: "qwen-turbo".into(),
            provider: "qwen".into(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            api_key_env: "DASHSCOPE_API_KEY".into(),
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.3,
                output_per_million: 0.6,
                cache_read_per_million: 0.1,
                cache_write_per_million: 0.0,
            },
            compat: default_compat(),
        },
        // ── Doubao ──
        Model {
            id: "doubao-1.5-pro-32k".into(),
            provider: "doubao".into(),
            base_url: "https://ark.cn-beijing.volces.com/api/v3".into(),
            api_key_env: "ARK_API_KEY".into(),
            max_tokens: 4096,
            context_window: 32768,
            cost: ModelCost {
                input_per_million: 0.8,
                output_per_million: 2.0,
                cache_read_per_million: 0.2,
                cache_write_per_million: 0.0,
            },
            compat: default_compat(),
        },
        Model {
            id: "doubao-1.5-pro-256k".into(),
            provider: "doubao".into(),
            base_url: "https://ark.cn-beijing.volces.com/api/v3".into(),
            api_key_env: "ARK_API_KEY".into(),
            max_tokens: 4096,
            context_window: 262144,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 9.0,
                cache_read_per_million: 1.0,
                cache_write_per_million: 0.0,
            },
            compat: default_compat(),
        },
        // ── Kimi ──
        Model {
            id: "moonshot-v1-auto".into(),
            provider: "kimi".into(),
            base_url: "https://api.moonshot.cn/v1".into(),
            api_key_env: "MOONSHOT_API_KEY".into(),
            max_tokens: 4096,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 3.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            compat: default_compat(),
        },
        Model {
            id: "moonshot-v1-8k".into(),
            provider: "kimi".into(),
            base_url: "https://api.moonshot.cn/v1".into(),
            api_key_env: "MOONSHOT_API_KEY".into(),
            max_tokens: 4096,
            context_window: 8192,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 3.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            compat: default_compat(),
        },
        // ── MiniMax ──
        Model {
            id: "MiniMax-M1".into(),
            provider: "minimax".into(),
            base_url: "https://api.minimax.chat/v1".into(),
            api_key_env: "MINIMAX_API_KEY".into(),
            max_tokens: 8192,
            context_window: 1048576,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 8.0,
                cache_read_per_million: 0.1,
                cache_write_per_million: 0.0,
            },
            compat: default_compat(),
        },
        Model {
            id: "MiniMax-Text-01".into(),
            provider: "minimax".into(),
            base_url: "https://api.minimax.chat/v1".into(),
            api_key_env: "MINIMAX_API_KEY".into(),
            max_tokens: 4096,
            context_window: 245760,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 5.0,
                cache_read_per_million: 0.1,
                cache_write_per_million: 0.0,
            },
            compat: default_compat(),
        },
        // ── ZAI (Zhipu) ──
        Model {
            id: "glm-4-plus".into(),
            provider: "zai".into(),
            base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
            api_key_env: "ZHIPU_API_KEY".into(),
            max_tokens: 4096,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 5.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            compat: ProviderCompat {
                max_tokens_field: MaxTokensField::MaxTokens,
                supports_reasoning_effort: false,
                thinking_format: Some(ThinkingFormat::Zai),
                requires_tool_result_name: true,
                requires_assistant_after_tool_result: true,
                supports_strict_mode: false,
            },
        },
        Model {
            id: "glm-4".into(),
            provider: "zai".into(),
            base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
            api_key_env: "ZHIPU_API_KEY".into(),
            max_tokens: 4096,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 1.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            compat: ProviderCompat {
                max_tokens_field: MaxTokensField::MaxTokens,
                supports_reasoning_effort: false,
                thinking_format: None,
                requires_tool_result_name: true,
                requires_assistant_after_tool_result: true,
                supports_strict_mode: false,
            },
        },
        // ── DeepSeek ──
        Model {
            id: "deepseek-chat".into(),
            provider: "deepseek".into(),
            base_url: "https://api.deepseek.com/v1".into(),
            api_key_env: "DEEPSEEK_API_KEY".into(),
            max_tokens: 8192,
            context_window: 65536,
            cost: ModelCost {
                input_per_million: 0.27,
                output_per_million: 1.1,
                cache_read_per_million: 0.07,
                cache_write_per_million: 0.0,
            },
            compat: default_compat(),
        },
        Model {
            id: "deepseek-reasoner".into(),
            provider: "deepseek".into(),
            base_url: "https://api.deepseek.com/v1".into(),
            api_key_env: "DEEPSEEK_API_KEY".into(),
            max_tokens: 16384,
            context_window: 65536,
            cost: ModelCost {
                input_per_million: 0.55,
                output_per_million: 2.19,
                cache_read_per_million: 0.14,
                cache_write_per_million: 0.0,
            },
            compat: ProviderCompat {
                max_tokens_field: MaxTokensField::MaxCompletionTokens,
                supports_reasoning_effort: true,
                thinking_format: Some(ThinkingFormat::OpenAI),
                requires_tool_result_name: false,
                requires_assistant_after_tool_result: false,
                supports_strict_mode: false,
            },
        },
    ]
});

fn default_compat() -> ProviderCompat {
    ProviderCompat {
        max_tokens_field: MaxTokensField::MaxTokens,
        supports_reasoning_effort: false,
        thinking_format: None,
        requires_tool_result_name: false,
        requires_assistant_after_tool_result: false,
        supports_strict_mode: false,
    }
}

/// Resolves a model by provider and model ID.
pub fn resolve_model(provider: &str, model_id: &str) -> Option<Model> {
    MODEL_CATALOG
        .iter()
        .find(|m| m.provider == provider && m.id == model_id)
        .cloned()
}

/// Returns all built-in models.
pub fn list_models() -> &'static [Model] {
    &MODEL_CATALOG
}

/// Returns all supported provider names.
pub fn list_providers() -> Vec<&'static str> {
    vec!["qwen", "doubao", "kimi", "minimax", "zai", "deepseek"]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::*;

    // ========================================================================
    // resolve_model
    // ========================================================================

    #[test]
    fn test_resolve_qwen_plus() {
        let model = resolve_model("qwen", "qwen-plus").unwrap();
        assert_eq!(model.id, "qwen-plus");
        assert_eq!(model.provider, "qwen");
        assert!(!model.base_url.is_empty());
        assert_eq!(model.api_key_env, "DASHSCOPE_API_KEY");
    }

    #[test]
    fn test_resolve_qwen_max() {
        let model = resolve_model("qwen", "qwen-max").unwrap();
        assert_eq!(model.id, "qwen-max");
        assert_eq!(model.provider, "qwen");
    }

    #[test]
    fn test_resolve_qwen_turbo() {
        let model = resolve_model("qwen", "qwen-turbo").unwrap();
        assert_eq!(model.id, "qwen-turbo");
    }

    #[test]
    fn test_resolve_doubao() {
        let model = resolve_model("doubao", "doubao-1.5-pro-32k").unwrap();
        assert_eq!(model.id, "doubao-1.5-pro-32k");
        assert_eq!(model.provider, "doubao");
        assert_eq!(model.api_key_env, "ARK_API_KEY");
    }

    #[test]
    fn test_resolve_kimi() {
        let model = resolve_model("kimi", "moonshot-v1-auto").unwrap();
        assert_eq!(model.id, "moonshot-v1-auto");
        assert_eq!(model.provider, "kimi");
        assert_eq!(model.api_key_env, "MOONSHOT_API_KEY");
    }

    #[test]
    fn test_resolve_minimax() {
        let model = resolve_model("minimax", "MiniMax-M1").unwrap();
        assert_eq!(model.id, "MiniMax-M1");
        assert_eq!(model.provider, "minimax");
        assert_eq!(model.api_key_env, "MINIMAX_API_KEY");
    }

    #[test]
    fn test_resolve_zai() {
        let model = resolve_model("zai", "glm-4-plus").unwrap();
        assert_eq!(model.id, "glm-4-plus");
        assert_eq!(model.provider, "zai");
        assert_eq!(model.api_key_env, "ZHIPU_API_KEY");
    }

    #[test]
    fn test_resolve_deepseek_chat() {
        let model = resolve_model("deepseek", "deepseek-chat").unwrap();
        assert_eq!(model.id, "deepseek-chat");
        assert_eq!(model.provider, "deepseek");
        assert_eq!(model.api_key_env, "DEEPSEEK_API_KEY");
    }

    #[test]
    fn test_resolve_deepseek_reasoner() {
        let model = resolve_model("deepseek", "deepseek-reasoner").unwrap();
        assert_eq!(model.id, "deepseek-reasoner");
    }

    #[test]
    fn test_resolve_unknown_model_returns_none() {
        assert!(resolve_model("qwen", "nonexistent-model").is_none());
    }

    #[test]
    fn test_resolve_unknown_provider_returns_none() {
        assert!(resolve_model("openai", "gpt-4").is_none());
    }

    // ========================================================================
    // list_models / list_providers
    // ========================================================================

    #[test]
    fn test_list_models_not_empty() {
        let models = list_models();
        assert!(
            models.len() >= 10,
            "should have at least 10 built-in models"
        );
    }

    #[test]
    fn test_list_providers_all_six() {
        let providers = list_providers();
        let expected = ["qwen", "doubao", "kimi", "minimax", "zai", "deepseek"];
        for p in &expected {
            assert!(providers.contains(p), "missing provider: {}", p);
        }
    }

    #[test]
    fn test_every_provider_has_at_least_one_model() {
        let providers = list_providers();
        for p in &providers {
            let models: Vec<_> = list_models().iter().filter(|m| m.provider == *p).collect();
            assert!(!models.is_empty(), "provider {} has no models", p);
        }
    }

    #[test]
    fn test_all_models_have_positive_context_window() {
        for model in list_models() {
            assert!(
                model.context_window > 0,
                "model {} has zero context_window",
                model.id
            );
        }
    }

    // ========================================================================
    // resolve_model — case sensitivity
    // ========================================================================

    #[test]
    fn test_resolve_model_case_sensitive_provider_capital() {
        // Provider names should be strictly matched — "Qwen" != "qwen"
        assert!(
            resolve_model("Qwen", "qwen-plus").is_none(),
            "provider lookup should be case-sensitive"
        );
    }

    #[test]
    fn test_resolve_model_case_sensitive_model_upper() {
        // Model ID "QWEN-PLUS" != "qwen-plus"
        assert!(
            resolve_model("qwen", "QWEN-PLUS").is_none(),
            "model id lookup should be case-sensitive"
        );
    }

    #[test]
    fn test_resolve_model_case_sensitive_mixed() {
        // "Qwen-Plus" is not "qwen-plus"
        assert!(
            resolve_model("qwen", "Qwen-Plus").is_none(),
            "mixed-case model id should not match"
        );
    }

    // ========================================================================
    // Model compat flags correctness
    // ========================================================================

    #[test]
    fn test_deepseek_reasoner_compat_flags() {
        let model = resolve_model("deepseek", "deepseek-reasoner").unwrap();
        // deepseek-reasoner should use MaxCompletionTokens and have a thinking_format
        assert!(
            matches!(
                model.compat.max_tokens_field,
                MaxTokensField::MaxCompletionTokens
            ),
            "deepseek-reasoner should use max_completion_tokens"
        );
        assert!(
            model.compat.thinking_format.is_some(),
            "deepseek-reasoner should have a thinking_format"
        );
    }

    #[test]
    fn test_deepseek_chat_no_thinking() {
        let model = resolve_model("deepseek", "deepseek-chat").unwrap();
        // deepseek-chat (non-reasoner) should not have thinking_format
        assert!(
            model.compat.thinking_format.is_none(),
            "deepseek-chat should not have thinking_format"
        );
    }

    #[test]
    fn test_zai_compat_requires_tool_result_name() {
        let model = resolve_model("zai", "glm-4-plus").unwrap();
        assert!(
            model.compat.requires_tool_result_name,
            "zai provider should require tool_result name"
        );
    }

    // ========================================================================
    // list_models — no duplicate IDs
    // ========================================================================

    #[test]
    fn test_list_models_no_duplicate_ids() {
        let models = list_models();
        let mut seen = std::collections::HashSet::new();
        for model in models {
            assert!(
                seen.insert(&model.id),
                "duplicate model id found: {}",
                model.id
            );
        }
    }

    // ========================================================================
    // Model max_tokens <= context_window
    // ========================================================================

    #[test]
    fn test_all_models_max_tokens_within_context_window() {
        for model in list_models() {
            assert!(
                model.max_tokens <= model.context_window,
                "model {} has max_tokens ({}) > context_window ({})",
                model.id,
                model.max_tokens,
                model.context_window
            );
        }
    }

    // ========================================================================
    // 边界: 空字符串 provider / model_id
    // ========================================================================

    #[test]
    fn test_resolve_model_empty_provider() {
        assert!(
            resolve_model("", "qwen-plus").is_none(),
            "empty provider should return None"
        );
    }

    #[test]
    fn test_resolve_model_empty_model_id() {
        assert!(
            resolve_model("qwen", "").is_none(),
            "empty model_id should return None"
        );
    }

    #[test]
    fn test_resolve_model_both_empty() {
        assert!(
            resolve_model("", "").is_none(),
            "both empty should return None"
        );
    }
}
