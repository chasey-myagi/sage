// Built-in model catalog — Phase 2
// Registry of known LLM models and their configurations.

use std::sync::LazyLock;

use super::types::*;

static MODEL_CATALOG: LazyLock<Vec<Model>> = LazyLock::new(|| {
    vec![
        // ── Anthropic ──
        Model {
            id: "claude-opus-4-20250514".into(),
            name: "Claude Opus 4".into(),
            api: api::ANTHROPIC_MESSAGES.into(),
            provider: provider::ANTHROPIC.into(),
            base_url: "https://api.anthropic.com/v1".into(),
            api_key_env: "ANTHROPIC_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 32000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 15.0,
                output_per_million: 75.0,
                cache_read_per_million: 1.5,
                cache_write_per_million: 18.75,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "claude-sonnet-4-20250514".into(),
            name: "Claude Sonnet 4".into(),
            api: api::ANTHROPIC_MESSAGES.into(),
            provider: provider::ANTHROPIC.into(),
            base_url: "https://api.anthropic.com/v1".into(),
            api_key_env: "ANTHROPIC_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cache_read_per_million: 0.3,
                cache_write_per_million: 3.75,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "claude-3-5-sonnet-20241022".into(),
            name: "Claude 3.5 Sonnet".into(),
            api: api::ANTHROPIC_MESSAGES.into(),
            provider: provider::ANTHROPIC.into(),
            base_url: "https://api.anthropic.com/v1".into(),
            api_key_env: "ANTHROPIC_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 8192,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cache_read_per_million: 0.3,
                cache_write_per_million: 3.75,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "claude-3-5-haiku-20241022".into(),
            name: "Claude 3.5 Haiku".into(),
            api: api::ANTHROPIC_MESSAGES.into(),
            provider: provider::ANTHROPIC.into(),
            base_url: "https://api.anthropic.com/v1".into(),
            api_key_env: "ANTHROPIC_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 8192,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 0.8,
                output_per_million: 4.0,
                cache_read_per_million: 0.08,
                cache_write_per_million: 1.0,
            },
            headers: vec![],
            compat: None,
        },
        // ── OpenAI (Chat Completions) ──
        Model {
            id: "gpt-4.1".into(),
            name: "GPT-4.1".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::OPENAI.into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 32768,
            context_window: 1047576,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 8.0,
                cache_read_per_million: 0.5,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: Some(ProviderCompat {
                max_tokens_field: MaxTokensField::MaxCompletionTokens,
                supports_strict_mode: true,
                ..ProviderCompat::default()
            }),
        },
        Model {
            id: "gpt-4.1-mini".into(),
            name: "GPT-4.1 Mini".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::OPENAI.into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 32768,
            context_window: 1047576,
            cost: ModelCost {
                input_per_million: 0.4,
                output_per_million: 1.6,
                cache_read_per_million: 0.1,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: Some(ProviderCompat {
                max_tokens_field: MaxTokensField::MaxCompletionTokens,
                supports_strict_mode: true,
                ..ProviderCompat::default()
            }),
        },
        Model {
            id: "gpt-4.1-nano".into(),
            name: "GPT-4.1 Nano".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::OPENAI.into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 32768,
            context_window: 1047576,
            cost: ModelCost {
                input_per_million: 0.1,
                output_per_million: 0.4,
                cache_read_per_million: 0.025,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: Some(ProviderCompat {
                max_tokens_field: MaxTokensField::MaxCompletionTokens,
                supports_strict_mode: true,
                ..ProviderCompat::default()
            }),
        },
        Model {
            id: "o3".into(),
            name: "o3".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::OPENAI.into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 100000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 10.0,
                output_per_million: 40.0,
                cache_read_per_million: 2.5,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: Some(ProviderCompat {
                max_tokens_field: MaxTokensField::MaxCompletionTokens,
                supports_reasoning_effort: true,
                thinking_format: Some(ThinkingFormat::OpenAI),
                supports_strict_mode: true,
                ..ProviderCompat::default()
            }),
        },
        Model {
            id: "o4-mini".into(),
            name: "o4-mini".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::OPENAI.into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 100000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 1.1,
                output_per_million: 4.4,
                cache_read_per_million: 0.275,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: Some(ProviderCompat {
                max_tokens_field: MaxTokensField::MaxCompletionTokens,
                supports_reasoning_effort: true,
                thinking_format: Some(ThinkingFormat::OpenAI),
                supports_strict_mode: true,
                ..ProviderCompat::default()
            }),
        },
        // ── Google ──
        Model {
            id: "gemini-2.5-pro".into(),
            name: "Gemini 2.5 Pro".into(),
            api: api::GOOGLE_GENERATIVE_AI.into(),
            provider: provider::GOOGLE.into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            api_key_env: "GOOGLE_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 65536,
            context_window: 1048576,
            cost: ModelCost {
                input_per_million: 1.25,
                output_per_million: 10.0,
                cache_read_per_million: 0.315,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gemini-2.5-flash".into(),
            name: "Gemini 2.5 Flash".into(),
            api: api::GOOGLE_GENERATIVE_AI.into(),
            provider: provider::GOOGLE.into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            api_key_env: "GOOGLE_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 65536,
            context_window: 1048576,
            cost: ModelCost {
                input_per_million: 0.15,
                output_per_million: 0.6,
                cache_read_per_million: 0.0375,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gemini-2.0-flash".into(),
            name: "Gemini 2.0 Flash".into(),
            api: api::GOOGLE_GENERATIVE_AI.into(),
            provider: provider::GOOGLE.into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            api_key_env: "GOOGLE_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 8192,
            context_window: 1048576,
            cost: ModelCost {
                input_per_million: 0.1,
                output_per_million: 0.4,
                cache_read_per_million: 0.025,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ── Qwen ──
        Model {
            id: "qwen-plus".into(),
            name: "Qwen Plus".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::QWEN.into(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            api_key_env: "DASHSCOPE_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.8,
                output_per_million: 2.0,
                cache_read_per_million: 0.2,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: default_compat(),
        },
        Model {
            id: "qwen-max".into(),
            name: "Qwen Max".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::QWEN.into(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            api_key_env: "DASHSCOPE_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 32768,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 6.0,
                cache_read_per_million: 0.5,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: default_compat(),
        },
        Model {
            id: "qwen-turbo".into(),
            name: "Qwen Turbo".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::QWEN.into(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            api_key_env: "DASHSCOPE_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.3,
                output_per_million: 0.6,
                cache_read_per_million: 0.1,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: default_compat(),
        },
        // ── Doubao ──
        Model {
            id: "doubao-1.5-pro-32k".into(),
            name: "Doubao 1.5 Pro 32K".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::DOUBAO.into(),
            base_url: "https://ark.cn-beijing.volces.com/api/v3".into(),
            api_key_env: "ARK_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 32768,
            cost: ModelCost {
                input_per_million: 0.8,
                output_per_million: 2.0,
                cache_read_per_million: 0.2,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: default_compat(),
        },
        Model {
            id: "doubao-1.5-pro-256k".into(),
            name: "Doubao 1.5 Pro 256K".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::DOUBAO.into(),
            base_url: "https://ark.cn-beijing.volces.com/api/v3".into(),
            api_key_env: "ARK_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 262144,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 9.0,
                cache_read_per_million: 1.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: default_compat(),
        },
        // ── Kimi ──
        Model {
            id: "moonshot-v1-auto".into(),
            name: "Moonshot v1 Auto".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::KIMI.into(),
            base_url: "https://api.moonshot.cn/v1".into(),
            api_key_env: "MOONSHOT_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 3.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: default_compat(),
        },
        Model {
            id: "moonshot-v1-8k".into(),
            name: "Moonshot v1 8K".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::KIMI.into(),
            base_url: "https://api.moonshot.cn/v1".into(),
            api_key_env: "MOONSHOT_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 8192,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 3.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: default_compat(),
        },
        // ── MiniMax ──
        Model {
            id: "MiniMax-M1".into(),
            name: "MiniMax M1".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MINIMAX.into(),
            base_url: "https://api.minimax.chat/v1".into(),
            api_key_env: "MINIMAX_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 1048576,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 8.0,
                cache_read_per_million: 0.1,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: default_compat(),
        },
        Model {
            id: "MiniMax-Text-01".into(),
            name: "MiniMax Text 01".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MINIMAX.into(),
            base_url: "https://api.minimax.chat/v1".into(),
            api_key_env: "MINIMAX_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 245760,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 5.0,
                cache_read_per_million: 0.1,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: default_compat(),
        },
        // ── ZAI (Zhipu) ──
        Model {
            id: "glm-4-plus".into(),
            name: "GLM-4 Plus".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::ZAI.into(),
            base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
            api_key_env: "ZHIPU_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 5.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: Some(ProviderCompat {
                thinking_format: Some(ThinkingFormat::Zai),
                requires_tool_result_name: true,
                requires_assistant_after_tool_result: true,
                supports_strict_mode: false,
                supports_store: false,
                supports_developer_role: false,
                ..ProviderCompat::default()
            }),
        },
        Model {
            id: "glm-4".into(),
            name: "GLM-4".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::ZAI.into(),
            base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
            api_key_env: "ZHIPU_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 1.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: Some(ProviderCompat {
                requires_tool_result_name: true,
                requires_assistant_after_tool_result: true,
                supports_strict_mode: false,
                supports_store: false,
                supports_developer_role: false,
                ..ProviderCompat::default()
            }),
        },
        // ── DeepSeek ──
        Model {
            id: "deepseek-chat".into(),
            name: "DeepSeek Chat".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::DEEPSEEK.into(),
            base_url: "https://api.deepseek.com/v1".into(),
            api_key_env: "DEEPSEEK_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 65536,
            cost: ModelCost {
                input_per_million: 0.27,
                output_per_million: 1.1,
                cache_read_per_million: 0.07,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: default_compat(),
        },
        Model {
            id: "deepseek-reasoner".into(),
            name: "DeepSeek Reasoner".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::DEEPSEEK.into(),
            base_url: "https://api.deepseek.com/v1".into(),
            api_key_env: "DEEPSEEK_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 16384,
            context_window: 65536,
            cost: ModelCost {
                input_per_million: 0.55,
                output_per_million: 2.19,
                cache_read_per_million: 0.14,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: Some(ProviderCompat {
                max_tokens_field: MaxTokensField::MaxCompletionTokens,
                supports_reasoning_effort: true,
                thinking_format: Some(ThinkingFormat::OpenAI),
                supports_strict_mode: false,
                supports_store: false,
                supports_developer_role: false,
                ..ProviderCompat::default()
            }),
        },
    ]
});

fn default_compat() -> Option<ProviderCompat> {
    Some(ProviderCompat::default())
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
    vec![
        "anthropic",
        "openai",
        "google",
        "qwen",
        "doubao",
        "kimi",
        "minimax",
        "zai",
        "deepseek",
    ]
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
        assert!(resolve_model("nonexistent-provider", "some-model").is_none());
    }

    // ========================================================================
    // list_models / list_providers
    // ========================================================================

    #[test]
    fn test_list_models_not_empty() {
        let models = list_models();
        assert!(
            models.len() >= 22,
            "should have at least 22 built-in models, got {}",
            models.len()
        );
    }

    #[test]
    fn test_list_providers_all_nine() {
        let providers = list_providers();
        let expected = [
            "anthropic",
            "openai",
            "google",
            "qwen",
            "doubao",
            "kimi",
            "minimax",
            "zai",
            "deepseek",
        ];
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
        let compat = model.compat.as_ref().unwrap();
        // deepseek-reasoner should use MaxCompletionTokens and have a thinking_format
        assert!(
            matches!(compat.max_tokens_field, MaxTokensField::MaxCompletionTokens),
            "deepseek-reasoner should use max_completion_tokens"
        );
        assert!(
            compat.thinking_format.is_some(),
            "deepseek-reasoner should have a thinking_format"
        );
    }

    #[test]
    fn test_deepseek_chat_no_thinking() {
        let model = resolve_model("deepseek", "deepseek-chat").unwrap();
        let compat = model.compat.as_ref().unwrap();
        // deepseek-chat (non-reasoner) should not have thinking_format
        assert!(
            compat.thinking_format.is_none(),
            "deepseek-chat should not have thinking_format"
        );
    }

    #[test]
    fn test_zai_compat_requires_tool_result_name() {
        let model = resolve_model("zai", "glm-4-plus").unwrap();
        let compat = model.compat.as_ref().unwrap();
        assert!(
            compat.requires_tool_result_name,
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

    // ========================================================================
    // resolve_model — Anthropic models
    // ========================================================================

    #[test]
    fn test_resolve_anthropic_opus() {
        let model = resolve_model("anthropic", "claude-opus-4-20250514").unwrap();
        assert_eq!(model.id, "claude-opus-4-20250514");
        assert_eq!(model.name, "Claude Opus 4");
        assert_eq!(model.provider, "anthropic");
        assert_eq!(model.api, api::ANTHROPIC_MESSAGES);
        assert_eq!(model.api_key_env, "ANTHROPIC_API_KEY");
        assert!(model.reasoning);
        assert!(model.input.contains(&InputType::Image));
        assert!(model.compat.is_none());
    }

    #[test]
    fn test_resolve_anthropic_sonnet4() {
        let model = resolve_model("anthropic", "claude-sonnet-4-20250514").unwrap();
        assert_eq!(model.id, "claude-sonnet-4-20250514");
        assert!(model.reasoning);
        assert_eq!(model.max_tokens, 16000);
    }

    #[test]
    fn test_resolve_anthropic_sonnet35() {
        let model = resolve_model("anthropic", "claude-3-5-sonnet-20241022").unwrap();
        assert_eq!(model.id, "claude-3-5-sonnet-20241022");
        assert!(!model.reasoning);
    }

    #[test]
    fn test_resolve_anthropic_haiku35() {
        let model = resolve_model("anthropic", "claude-3-5-haiku-20241022").unwrap();
        assert_eq!(model.id, "claude-3-5-haiku-20241022");
        assert!(!model.reasoning);
        assert_eq!(model.context_window, 200000);
    }

    // ========================================================================
    // resolve_model — OpenAI models
    // ========================================================================

    #[test]
    fn test_resolve_openai_gpt41() {
        let model = resolve_model("openai", "gpt-4.1").unwrap();
        assert_eq!(model.id, "gpt-4.1");
        assert_eq!(model.name, "GPT-4.1");
        assert_eq!(model.provider, "openai");
        assert_eq!(model.api, api::OPENAI_COMPLETIONS);
        assert_eq!(model.api_key_env, "OPENAI_API_KEY");
        assert!(!model.reasoning);
        let compat = model.compat.as_ref().unwrap();
        assert!(matches!(
            compat.max_tokens_field,
            MaxTokensField::MaxCompletionTokens
        ));
        assert!(compat.supports_strict_mode);
    }

    #[test]
    fn test_resolve_openai_gpt41_mini() {
        let model = resolve_model("openai", "gpt-4.1-mini").unwrap();
        assert_eq!(model.id, "gpt-4.1-mini");
        assert!(!model.reasoning);
    }

    #[test]
    fn test_resolve_openai_gpt41_nano() {
        let model = resolve_model("openai", "gpt-4.1-nano").unwrap();
        assert_eq!(model.id, "gpt-4.1-nano");
        assert!(!model.reasoning);
    }

    #[test]
    fn test_resolve_openai_o3() {
        let model = resolve_model("openai", "o3").unwrap();
        assert_eq!(model.id, "o3");
        assert!(model.reasoning);
        let compat = model.compat.as_ref().unwrap();
        assert!(compat.supports_reasoning_effort);
        assert!(matches!(
            compat.thinking_format,
            Some(ThinkingFormat::OpenAI)
        ));
        assert!(compat.supports_strict_mode);
    }

    #[test]
    fn test_resolve_openai_o4_mini() {
        let model = resolve_model("openai", "o4-mini").unwrap();
        assert_eq!(model.id, "o4-mini");
        assert!(model.reasoning);
        let compat = model.compat.as_ref().unwrap();
        assert!(compat.supports_reasoning_effort);
    }

    // ========================================================================
    // resolve_model — Google models
    // ========================================================================

    #[test]
    fn test_resolve_google_gemini25_pro() {
        let model = resolve_model("google", "gemini-2.5-pro").unwrap();
        assert_eq!(model.id, "gemini-2.5-pro");
        assert_eq!(model.name, "Gemini 2.5 Pro");
        assert_eq!(model.provider, "google");
        assert_eq!(model.api, api::GOOGLE_GENERATIVE_AI);
        assert_eq!(model.api_key_env, "GOOGLE_API_KEY");
        assert!(model.reasoning);
        assert!(model.input.contains(&InputType::Image));
        assert!(model.compat.is_none());
    }

    #[test]
    fn test_resolve_google_gemini25_flash() {
        let model = resolve_model("google", "gemini-2.5-flash").unwrap();
        assert_eq!(model.id, "gemini-2.5-flash");
        assert!(model.reasoning);
        assert_eq!(model.context_window, 1048576);
    }

    #[test]
    fn test_resolve_google_gemini20_flash() {
        let model = resolve_model("google", "gemini-2.0-flash").unwrap();
        assert_eq!(model.id, "gemini-2.0-flash");
        assert!(!model.reasoning);
        assert_eq!(model.max_tokens, 8192);
    }
}
