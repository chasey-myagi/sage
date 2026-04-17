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
        // Default Qwen model (DashScope direct, Qwen3 generation).
        Model {
            id: "qwen3.6-plus".into(),
            name: "Qwen 3.6 Plus".into(),
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
        // Default Kimi model (Moonshot direct, K2.5 generation).
        Model {
            id: "kimi-k2.5".into(),
            name: "Kimi K2.5".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::KIMI.into(),
            base_url: "https://api.moonshot.cn/v1".into(),
            api_key_env: "MOONSHOT_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 262144,
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
        // ── Groq ──
        // Groq compat flags are handled dynamically by detect_compat() in
        // openai_completions.rs (is_groq detection, reasoningEffortMap for
        // Qwen3-32b). Model entries here use compat: None so detect_compat
        // kicks in at request time.
        // Groq does not support prompt caching; cache costs are 0.
        Model {
            id: "deepseek-r1-distill-llama-70b".into(),
            name: "DeepSeek R1 Distill Llama 70B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GROQ.into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key_env: "GROQ_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.75,
                output_per_million: 0.99,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gemma2-9b-it".into(),
            name: "Gemma 2 9B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GROQ.into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key_env: "GROQ_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 8192,
            cost: ModelCost {
                input_per_million: 0.2,
                output_per_million: 0.2,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "llama-3.1-8b-instant".into(),
            name: "Llama 3.1 8B Instant".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GROQ.into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key_env: "GROQ_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 131072,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.05,
                output_per_million: 0.08,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "llama-3.3-70b-versatile".into(),
            name: "Llama 3.3 70B Versatile".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GROQ.into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key_env: "GROQ_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 32768,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.59,
                output_per_million: 0.79,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "llama3-70b-8192".into(),
            name: "Llama 3 70B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GROQ.into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key_env: "GROQ_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 8192,
            cost: ModelCost {
                input_per_million: 0.59,
                output_per_million: 0.79,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "llama3-8b-8192".into(),
            name: "Llama 3 8B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GROQ.into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key_env: "GROQ_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 8192,
            cost: ModelCost {
                input_per_million: 0.05,
                output_per_million: 0.08,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "meta-llama/llama-4-maverick-17b-128e-instruct".into(),
            name: "Llama 4 Maverick 17B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GROQ.into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key_env: "GROQ_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.2,
                output_per_million: 0.6,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "meta-llama/llama-4-scout-17b-16e-instruct".into(),
            name: "Llama 4 Scout 17B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GROQ.into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key_env: "GROQ_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.11,
                output_per_million: 0.34,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral-saba-24b".into(),
            name: "Mistral Saba 24B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GROQ.into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key_env: "GROQ_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 32768,
            context_window: 32768,
            cost: ModelCost {
                input_per_million: 0.79,
                output_per_million: 0.79,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "moonshotai/kimi-k2-instruct".into(),
            name: "Kimi K2 Instruct".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GROQ.into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key_env: "GROQ_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 16384,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 3.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "moonshotai/kimi-k2-instruct-0905".into(),
            name: "Kimi K2 Instruct 0905".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GROQ.into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key_env: "GROQ_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 16384,
            context_window: 262144,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 3.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "openai/gpt-oss-120b".into(),
            name: "GPT OSS 120B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GROQ.into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key_env: "GROQ_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 65536,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.15,
                output_per_million: 0.6,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "openai/gpt-oss-20b".into(),
            name: "GPT OSS 20B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GROQ.into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key_env: "GROQ_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 65536,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.075,
                output_per_million: 0.3,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "qwen-qwq-32b".into(),
            name: "Qwen QwQ 32B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GROQ.into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key_env: "GROQ_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 16384,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.29,
                output_per_million: 0.39,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "qwen/qwen3-32b".into(),
            name: "Qwen3 32B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GROQ.into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key_env: "GROQ_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 16384,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.29,
                output_per_million: 0.59,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ── xAI (Grok) ──
        // xAI compat flags are handled dynamically by detect_compat() in
        // openai_completions.rs (is_xai / is_grok detection, max_completion_tokens,
        // is_non_standard). Model entries use compat: None.
        // xAI supports prompt caching (cacheRead > 0 for most models).
        Model {
            id: "grok-2".into(),
            name: "Grok 2".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 10.0,
                cache_read_per_million: 2.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-2-1212".into(),
            name: "Grok 2 (1212)".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 10.0,
                cache_read_per_million: 2.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-2-latest".into(),
            name: "Grok 2 Latest".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 10.0,
                cache_read_per_million: 2.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-2-vision".into(),
            name: "Grok 2 Vision".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 4096,
            context_window: 8192,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 10.0,
                cache_read_per_million: 2.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-2-vision-1212".into(),
            name: "Grok 2 Vision (1212)".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 4096,
            context_window: 8192,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 10.0,
                cache_read_per_million: 2.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-2-vision-latest".into(),
            name: "Grok 2 Vision Latest".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 4096,
            context_window: 8192,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 10.0,
                cache_read_per_million: 2.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-3".into(),
            name: "Grok 3".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cache_read_per_million: 0.75,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-3-fast".into(),
            name: "Grok 3 Fast".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 25.0,
                cache_read_per_million: 1.25,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-3-fast-latest".into(),
            name: "Grok 3 Fast Latest".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 25.0,
                cache_read_per_million: 1.25,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-3-latest".into(),
            name: "Grok 3 Latest".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cache_read_per_million: 0.75,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-3-mini".into(),
            name: "Grok 3 Mini".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.3,
                output_per_million: 0.5,
                cache_read_per_million: 0.075,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-3-mini-fast".into(),
            name: "Grok 3 Mini Fast".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.6,
                output_per_million: 4.0,
                cache_read_per_million: 0.15,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-3-mini-fast-latest".into(),
            name: "Grok 3 Mini Fast Latest".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.6,
                output_per_million: 4.0,
                cache_read_per_million: 0.15,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-3-mini-latest".into(),
            name: "Grok 3 Mini Latest".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.3,
                output_per_million: 0.5,
                cache_read_per_million: 0.075,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-4".into(),
            name: "Grok 4".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 64000,
            context_window: 256000,
            cost: ModelCost {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cache_read_per_million: 0.75,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-4-1-fast".into(),
            name: "Grok 4.1 Fast".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 30000,
            context_window: 2000000,
            cost: ModelCost {
                input_per_million: 0.2,
                output_per_million: 0.5,
                cache_read_per_million: 0.05,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-4-1-fast-non-reasoning".into(),
            name: "Grok 4.1 Fast (Non-Reasoning)".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 30000,
            context_window: 2000000,
            cost: ModelCost {
                input_per_million: 0.2,
                output_per_million: 0.5,
                cache_read_per_million: 0.05,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-4-fast".into(),
            name: "Grok 4 Fast".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 30000,
            context_window: 2000000,
            cost: ModelCost {
                input_per_million: 0.2,
                output_per_million: 0.5,
                cache_read_per_million: 0.05,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-4-fast-non-reasoning".into(),
            name: "Grok 4 Fast (Non-Reasoning)".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 30000,
            context_window: 2000000,
            cost: ModelCost {
                input_per_million: 0.2,
                output_per_million: 0.5,
                cache_read_per_million: 0.05,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-4.20-0309-non-reasoning".into(),
            name: "Grok 4.20 (Non-Reasoning)".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 30000,
            context_window: 2000000,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 6.0,
                cache_read_per_million: 0.2,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-4.20-0309-reasoning".into(),
            name: "Grok 4.20 (Reasoning)".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 30000,
            context_window: 2000000,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 6.0,
                cache_read_per_million: 0.2,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-beta".into(),
            name: "Grok Beta".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 15.0,
                cache_read_per_million: 5.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-code-fast-1".into(),
            name: "Grok Code Fast 1".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 10000,
            context_window: 256000,
            cost: ModelCost {
                input_per_million: 0.2,
                output_per_million: 1.5,
                cache_read_per_million: 0.02,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "grok-vision-beta".into(),
            name: "Grok Vision Beta".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::XAI.into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 4096,
            context_window: 8192,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 15.0,
                cache_read_per_million: 5.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ── Cerebras ──
        // Cerebras compat handled by detect_compat() (is_non_standard).
        // Cerebras does not support prompt caching; cache costs are 0.
        Model {
            id: "gpt-oss-120b".into(),
            name: "GPT OSS 120B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::CEREBRAS.into(),
            base_url: "https://api.cerebras.ai/v1".into(),
            api_key_env: "CEREBRAS_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 32768,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.25,
                output_per_million: 0.69,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "llama3.1-8b".into(),
            name: "Llama 3.1 8B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::CEREBRAS.into(),
            base_url: "https://api.cerebras.ai/v1".into(),
            api_key_env: "CEREBRAS_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8000,
            context_window: 32000,
            cost: ModelCost {
                input_per_million: 0.1,
                output_per_million: 0.1,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "qwen-3-235b-a22b-instruct-2507".into(),
            name: "Qwen 3 235B Instruct".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::CEREBRAS.into(),
            base_url: "https://api.cerebras.ai/v1".into(),
            api_key_env: "CEREBRAS_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 32000,
            context_window: 131000,
            cost: ModelCost {
                input_per_million: 0.6,
                output_per_million: 1.2,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "zai-glm-4.7".into(),
            name: "Z.AI GLM-4.7".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::CEREBRAS.into(),
            base_url: "https://api.cerebras.ai/v1".into(),
            api_key_env: "CEREBRAS_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 40000,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 2.25,
                output_per_million: 2.75,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ====================================================================
        // Mistral — https://api.mistral.ai/v1
        // Mistral does not support prompt caching; cache costs are 0.
        // Pi-mono uses api: "mistral-conversations" with Mistral SDK;
        // we use openai-completions since Mistral HTTP API is OpenAI-compatible.
        // compat: None — standard OpenAI chat completions format, no special flags needed.
        // ====================================================================
        Model {
            id: "codestral-latest".into(),
            name: "Codestral (latest)".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 256000,
            cost: ModelCost {
                input_per_million: 0.3,
                output_per_million: 0.9,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "devstral-2512".into(),
            name: "Devstral 2".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 262144,
            context_window: 262144,
            cost: ModelCost {
                input_per_million: 0.4,
                output_per_million: 2.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "devstral-medium-2507".into(),
            name: "Devstral Medium".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 128000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.4,
                output_per_million: 2.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "devstral-medium-latest".into(),
            name: "Devstral 2 (latest)".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 262144,
            context_window: 262144,
            cost: ModelCost {
                input_per_million: 0.4,
                output_per_million: 2.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "devstral-small-2505".into(),
            name: "Devstral Small 2505".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 128000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.1,
                output_per_million: 0.3,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "devstral-small-2507".into(),
            name: "Devstral Small".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 128000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.1,
                output_per_million: 0.3,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // labs-* models are experimental and free (cost 0.0 per pi-mono).
        Model {
            id: "labs-devstral-small-2512".into(),
            name: "Devstral Small 2".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 256000,
            context_window: 256000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "magistral-medium-latest".into(),
            name: "Magistral Medium (latest)".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 16384,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 5.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "magistral-small".into(),
            name: "Magistral Small".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 128000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.5,
                output_per_million: 1.5,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "ministral-3b-latest".into(),
            name: "Ministral 3B (latest)".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 128000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.04,
                output_per_million: 0.04,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "ministral-8b-latest".into(),
            name: "Ministral 8B (latest)".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 128000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.1,
                output_per_million: 0.1,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral-large-2411".into(),
            name: "Mistral Large 2.1".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 16384,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 6.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral-large-2512".into(),
            name: "Mistral Large 3".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 262144,
            context_window: 262144,
            cost: ModelCost {
                input_per_million: 0.5,
                output_per_million: 1.5,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral-large-latest".into(),
            name: "Mistral Large (latest)".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 262144,
            context_window: 262144,
            cost: ModelCost {
                input_per_million: 0.5,
                output_per_million: 1.5,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral-medium-2505".into(),
            name: "Mistral Medium 3".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 131072,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.4,
                output_per_million: 2.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral-medium-2508".into(),
            name: "Mistral Medium 3.1".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 262144,
            context_window: 262144,
            cost: ModelCost {
                input_per_million: 0.4,
                output_per_million: 2.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral-medium-latest".into(),
            name: "Mistral Medium (latest)".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16384,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.4,
                output_per_million: 2.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral-nemo".into(),
            name: "Mistral Nemo".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 128000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.15,
                output_per_million: 0.15,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral-small-2506".into(),
            name: "Mistral Small 3.2".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16384,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.1,
                output_per_million: 0.3,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral-small-latest".into(),
            name: "Mistral Small (latest)".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16384,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.1,
                output_per_million: 0.3,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "open-mistral-7b".into(),
            name: "Mistral 7B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8000,
            context_window: 8000,
            cost: ModelCost {
                input_per_million: 0.25,
                output_per_million: 0.25,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "open-mixtral-8x22b".into(),
            name: "Mixtral 8x22B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 64000,
            context_window: 64000,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 6.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "open-mixtral-8x7b".into(),
            name: "Mixtral 8x7B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 32000,
            context_window: 32000,
            cost: ModelCost {
                input_per_million: 0.7,
                output_per_million: 0.7,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "pixtral-12b".into(),
            name: "Pixtral 12B".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.15,
                output_per_million: 0.15,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "pixtral-large-latest".into(),
            name: "Pixtral Large (latest)".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::MISTRAL.into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key_env: "MISTRAL_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 6.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ====================================================================
        // GitHub Copilot — https://api.individual.githubcopilot.com
        // All costs are 0 (Copilot is a paid subscription, not per-token).
        // Models span three API types: anthropic-messages, openai-completions,
        // openai-responses. The openai-completions models carry explicit compat
        // (supportsStore/DeveloperRole/ReasoningEffort = false) per pi-mono.
        // ====================================================================
        Model {
            id: "claude-haiku-4.5".into(),
            name: "Claude Haiku 4.5".into(),
            api: api::ANTHROPIC_MESSAGES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 32000,
            context_window: 144000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        Model {
            id: "claude-opus-4.5".into(),
            name: "Claude Opus 4.5".into(),
            api: api::ANTHROPIC_MESSAGES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 32000,
            context_window: 160000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        Model {
            id: "claude-opus-4.6".into(),
            name: "Claude Opus 4.6".into(),
            api: api::ANTHROPIC_MESSAGES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 1000000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        Model {
            id: "claude-sonnet-4".into(),
            name: "Claude Sonnet 4".into(),
            api: api::ANTHROPIC_MESSAGES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16000,
            context_window: 216000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        Model {
            id: "claude-sonnet-4.5".into(),
            name: "Claude Sonnet 4.5".into(),
            api: api::ANTHROPIC_MESSAGES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 32000,
            context_window: 144000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        Model {
            id: "claude-sonnet-4.6".into(),
            name: "Claude Sonnet 4.6".into(),
            api: api::ANTHROPIC_MESSAGES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 32000,
            context_window: 1000000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        // openai-completions models — explicit compat per pi-mono:
        // supportsStore=false, supportsDeveloperRole=false, supportsReasoningEffort=false
        Model {
            id: "gemini-2.5-pro".into(),
            name: "Gemini 2.5 Pro".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: Some(copilot_completions_compat()),
        },
        Model {
            id: "gemini-3-flash-preview".into(),
            name: "Gemini 3 Flash".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: Some(copilot_completions_compat()),
        },
        Model {
            id: "gemini-3-pro-preview".into(),
            name: "Gemini 3 Pro Preview".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: Some(copilot_completions_compat()),
        },
        Model {
            id: "gemini-3.1-pro-preview".into(),
            name: "Gemini 3.1 Pro Preview".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: Some(copilot_completions_compat()),
        },
        Model {
            id: "gpt-4.1".into(),
            name: "GPT-4.1".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16384,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: Some(copilot_completions_compat()),
        },
        Model {
            id: "gpt-4o".into(),
            name: "GPT-4o".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: Some(copilot_completions_compat()),
        },
        Model {
            id: "grok-code-fast-1".into(),
            name: "Grok Code Fast 1".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 64000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: Some(copilot_completions_compat()),
        },
        // openai-responses models
        Model {
            id: "gpt-5".into(),
            name: "GPT-5".into(),
            api: api::OPENAI_RESPONSES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        Model {
            id: "gpt-5-mini".into(),
            name: "GPT-5-mini".into(),
            api: api::OPENAI_RESPONSES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 264000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        Model {
            id: "gpt-5.1".into(),
            name: "GPT-5.1".into(),
            api: api::OPENAI_RESPONSES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 264000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        Model {
            id: "gpt-5.1-codex".into(),
            name: "GPT-5.1-Codex".into(),
            api: api::OPENAI_RESPONSES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        Model {
            id: "gpt-5.1-codex-max".into(),
            name: "GPT-5.1-Codex-max".into(),
            api: api::OPENAI_RESPONSES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        Model {
            id: "gpt-5.1-codex-mini".into(),
            name: "GPT-5.1-Codex-mini".into(),
            api: api::OPENAI_RESPONSES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        Model {
            id: "gpt-5.2".into(),
            name: "GPT-5.2".into(),
            api: api::OPENAI_RESPONSES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 264000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        Model {
            id: "gpt-5.2-codex".into(),
            name: "GPT-5.2-Codex".into(),
            api: api::OPENAI_RESPONSES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        Model {
            id: "gpt-5.3-codex".into(),
            name: "GPT-5.3-Codex".into(),
            api: api::OPENAI_RESPONSES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        Model {
            id: "gpt-5.4".into(),
            name: "GPT-5.4".into(),
            api: api::OPENAI_RESPONSES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        Model {
            id: "gpt-5.4-mini".into(),
            name: "GPT-5.4 mini".into(),
            api: api::OPENAI_RESPONSES.into(),
            provider: provider::GITHUB_COPILOT.into(),
            base_url: "https://api.individual.githubcopilot.com".into(),
            api_key_env: "COPILOT_GITHUB_TOKEN".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: copilot_headers(),
            compat: None,
        },
        // ====================================================================
        // Azure OpenAI Responses — 40 models (pi-mono: azure-openai-responses)
        // Azure-native models that use the Azure OpenAI Responses API.
        // Auth: api-key header; URL: resource-based; deployment name mapping.
        // All costs in $/M tokens. base_url is empty — resolved at runtime from
        // AZURE_OPENAI_BASE_URL or AZURE_OPENAI_RESOURCE_NAME env vars.
        // ====================================================================
        Model {
            id: "codex-mini-latest".into(),
            name: "Codex Mini".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 100000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 1.5,
                output_per_million: 6.0,
                cache_read_per_million: 0.375,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-4".into(),
            name: "GPT-4".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 8192,
            cost: ModelCost {
                input_per_million: 30.0,
                output_per_million: 60.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-4-turbo".into(),
            name: "GPT-4 Turbo".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 10.0,
                output_per_million: 30.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-4.1".into(),
            name: "GPT-4.1".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
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
            compat: None,
        },
        Model {
            id: "gpt-4.1-mini".into(),
            name: "GPT-4.1 mini".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
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
            compat: None,
        },
        Model {
            id: "gpt-4.1-nano".into(),
            name: "GPT-4.1 nano".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 32768,
            context_window: 1047576,
            cost: ModelCost {
                input_per_million: 0.1,
                output_per_million: 0.4,
                cache_read_per_million: 0.03,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-4o".into(),
            name: "GPT-4o".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16384,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 2.5,
                output_per_million: 10.0,
                cache_read_per_million: 1.25,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-4o-2024-05-13".into(),
            name: "GPT-4o (2024-05-13)".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 15.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-4o-2024-08-06".into(),
            name: "GPT-4o (2024-08-06)".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16384,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 2.5,
                output_per_million: 10.0,
                cache_read_per_million: 1.25,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-4o-2024-11-20".into(),
            name: "GPT-4o (2024-11-20)".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16384,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 2.5,
                output_per_million: 10.0,
                cache_read_per_million: 1.25,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-4o-mini".into(),
            name: "GPT-4o mini".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16384,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.15,
                output_per_million: 0.6,
                cache_read_per_million: 0.08,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5".into(),
            name: "GPT-5".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 1.25,
                output_per_million: 10.0,
                cache_read_per_million: 0.125,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5-chat-latest".into(),
            name: "GPT-5 Chat Latest".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16384,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 1.25,
                output_per_million: 10.0,
                cache_read_per_million: 0.125,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5-codex".into(),
            name: "GPT-5-Codex".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 1.25,
                output_per_million: 10.0,
                cache_read_per_million: 0.125,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5-mini".into(),
            name: "GPT-5 Mini".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 0.25,
                output_per_million: 2.0,
                cache_read_per_million: 0.025,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5-nano".into(),
            name: "GPT-5 Nano".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 0.05,
                output_per_million: 0.4,
                cache_read_per_million: 0.005,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5-pro".into(),
            name: "GPT-5 Pro".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 272000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 15.0,
                output_per_million: 120.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5.1".into(),
            name: "GPT-5.1".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 1.25,
                output_per_million: 10.0,
                cache_read_per_million: 0.13,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5.1-chat-latest".into(),
            name: "GPT-5.1 Chat".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16384,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 1.25,
                output_per_million: 10.0,
                cache_read_per_million: 0.125,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5.1-codex".into(),
            name: "GPT-5.1 Codex".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 1.25,
                output_per_million: 10.0,
                cache_read_per_million: 0.125,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5.1-codex-max".into(),
            name: "GPT-5.1 Codex Max".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 1.25,
                output_per_million: 10.0,
                cache_read_per_million: 0.125,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5.1-codex-mini".into(),
            name: "GPT-5.1 Codex mini".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 0.25,
                output_per_million: 2.0,
                cache_read_per_million: 0.025,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5.2".into(),
            name: "GPT-5.2".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 1.75,
                output_per_million: 14.0,
                cache_read_per_million: 0.175,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5.2-chat-latest".into(),
            name: "GPT-5.2 Chat".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16384,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 1.75,
                output_per_million: 14.0,
                cache_read_per_million: 0.175,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5.2-codex".into(),
            name: "GPT-5.2 Codex".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 1.75,
                output_per_million: 14.0,
                cache_read_per_million: 0.175,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5.2-pro".into(),
            name: "GPT-5.2 Pro".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 21.0,
                output_per_million: 168.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5.3-codex".into(),
            name: "GPT-5.3 Codex".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 1.75,
                output_per_million: 14.0,
                cache_read_per_million: 0.175,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5.3-codex-spark".into(),
            name: "GPT-5.3 Codex Spark".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 32000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 1.75,
                output_per_million: 14.0,
                cache_read_per_million: 0.175,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5.4".into(),
            name: "GPT-5.4".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 272000,
            cost: ModelCost {
                input_per_million: 2.5,
                output_per_million: 15.0,
                cache_read_per_million: 0.25,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5.4-mini".into(),
            name: "GPT-5.4 mini".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 0.75,
                output_per_million: 4.5,
                cache_read_per_million: 0.075,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5.4-nano".into(),
            name: "GPT-5.4 nano".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 400000,
            cost: ModelCost {
                input_per_million: 0.2,
                output_per_million: 1.25,
                cache_read_per_million: 0.02,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gpt-5.4-pro".into(),
            name: "GPT-5.4 Pro".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 1050000,
            cost: ModelCost {
                input_per_million: 30.0,
                output_per_million: 180.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "o1".into(),
            name: "o1".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 100000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 15.0,
                output_per_million: 60.0,
                cache_read_per_million: 7.5,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "o1-pro".into(),
            name: "o1-pro".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 100000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 150.0,
                output_per_million: 600.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "o3".into(),
            name: "o3".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 100000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 8.0,
                cache_read_per_million: 0.5,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "o3-deep-research".into(),
            name: "o3-deep-research".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
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
            compat: None,
        },
        Model {
            id: "o3-mini".into(),
            name: "o3-mini".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 100000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 1.1,
                output_per_million: 4.4,
                cache_read_per_million: 0.55,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "o3-pro".into(),
            name: "o3-pro".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 100000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 20.0,
                output_per_million: 80.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "o4-mini".into(),
            name: "o4-mini".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 100000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 1.1,
                output_per_million: 4.4,
                cache_read_per_million: 0.28,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "o4-mini-deep-research".into(),
            name: "o4-mini-deep-research".into(),
            api: api::AZURE_OPENAI_RESPONSES.into(),
            provider: provider::AZURE_OPENAI.into(),
            base_url: String::new(),
            api_key_env: "AZURE_OPENAI_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 100000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 8.0,
                cache_read_per_million: 0.5,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ── Amazon Bedrock ──
        // ─── Bedrock / Amazon Nova ───
        Model {
            id: "amazon.nova-2-lite-v1:0".into(),
            name: "Nova 2 Lite".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.33,
                output_per_million: 2.75,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "amazon.nova-lite-v1:0".into(),
            name: "Nova Lite".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 8192,
            context_window: 300000,
            cost: ModelCost {
                input_per_million: 0.06,
                output_per_million: 0.24,
                cache_read_per_million: 0.015,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "amazon.nova-micro-v1:0".into(),
            name: "Nova Micro".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.035,
                output_per_million: 0.14,
                cache_read_per_million: 0.00875,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "amazon.nova-premier-v1:0".into(),
            name: "Nova Premier".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16384,
            context_window: 1000000,
            cost: ModelCost {
                input_per_million: 2.5,
                output_per_million: 12.5,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "amazon.nova-pro-v1:0".into(),
            name: "Nova Pro".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 8192,
            context_window: 300000,
            cost: ModelCost {
                input_per_million: 0.8,
                output_per_million: 3.2,
                cache_read_per_million: 0.2,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ─── Bedrock / Anthropic Claude ───
        Model {
            id: "anthropic.claude-3-5-haiku-20241022-v1:0".into(),
            name: "Claude Haiku 3.5".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
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
        Model {
            id: "anthropic.claude-3-5-sonnet-20240620-v1:0".into(),
            name: "Claude Sonnet 3.5".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
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
            id: "anthropic.claude-3-5-sonnet-20241022-v2:0".into(),
            name: "Claude Sonnet 3.5 v2".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
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
            id: "anthropic.claude-3-7-sonnet-20250219-v1:0".into(),
            name: "Claude Sonnet 3.7".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
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
            id: "anthropic.claude-3-haiku-20240307-v1:0".into(),
            name: "Claude Haiku 3".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 4096,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 0.25,
                output_per_million: 1.25,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "anthropic.claude-haiku-4-5-20251001-v1:0".into(),
            name: "Claude Haiku 4.5".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 5.0,
                cache_read_per_million: 0.1,
                cache_write_per_million: 1.25,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "anthropic.claude-opus-4-1-20250805-v1:0".into(),
            name: "Claude Opus 4.1".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
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
            id: "anthropic.claude-opus-4-20250514-v1:0".into(),
            name: "Claude Opus 4".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
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
            id: "anthropic.claude-opus-4-5-20251101-v1:0".into(),
            name: "Claude Opus 4.5".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 25.0,
                cache_read_per_million: 0.5,
                cache_write_per_million: 6.25,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "anthropic.claude-opus-4-6-v1".into(),
            name: "Claude Opus 4.6".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 1000000,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 25.0,
                cache_read_per_million: 0.5,
                cache_write_per_million: 6.25,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "anthropic.claude-sonnet-4-20250514-v1:0".into(),
            name: "Claude Sonnet 4".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
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
            id: "anthropic.claude-sonnet-4-5-20250929-v1:0".into(),
            name: "Claude Sonnet 4.5".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
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
            id: "anthropic.claude-sonnet-4-6".into(),
            name: "Claude Sonnet 4.6".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 1000000,
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
            id: "eu.anthropic.claude-haiku-4-5-20251001-v1:0".into(),
            name: "Claude Haiku 4.5 (EU)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 5.0,
                cache_read_per_million: 0.1,
                cache_write_per_million: 1.25,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "eu.anthropic.claude-opus-4-5-20251101-v1:0".into(),
            name: "Claude Opus 4.5 (EU)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 25.0,
                cache_read_per_million: 0.5,
                cache_write_per_million: 6.25,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "eu.anthropic.claude-opus-4-6-v1".into(),
            name: "Claude Opus 4.6 (EU)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 1000000,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 25.0,
                cache_read_per_million: 0.5,
                cache_write_per_million: 6.25,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "eu.anthropic.claude-sonnet-4-20250514-v1:0".into(),
            name: "Claude Sonnet 4 (EU)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
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
            id: "eu.anthropic.claude-sonnet-4-5-20250929-v1:0".into(),
            name: "Claude Sonnet 4.5 (EU)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
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
            id: "eu.anthropic.claude-sonnet-4-6".into(),
            name: "Claude Sonnet 4.6 (EU)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 1000000,
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
            id: "global.anthropic.claude-haiku-4-5-20251001-v1:0".into(),
            name: "Claude Haiku 4.5 (Global)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 5.0,
                cache_read_per_million: 0.1,
                cache_write_per_million: 1.25,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "global.anthropic.claude-opus-4-5-20251101-v1:0".into(),
            name: "Claude Opus 4.5 (Global)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 25.0,
                cache_read_per_million: 0.5,
                cache_write_per_million: 6.25,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "global.anthropic.claude-opus-4-6-v1".into(),
            name: "Claude Opus 4.6 (Global)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 1000000,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 25.0,
                cache_read_per_million: 0.5,
                cache_write_per_million: 6.25,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "global.anthropic.claude-sonnet-4-20250514-v1:0".into(),
            name: "Claude Sonnet 4 (Global)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
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
            id: "global.anthropic.claude-sonnet-4-5-20250929-v1:0".into(),
            name: "Claude Sonnet 4.5 (Global)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
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
            id: "global.anthropic.claude-sonnet-4-6".into(),
            name: "Claude Sonnet 4.6 (Global)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 1000000,
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
            id: "us.anthropic.claude-haiku-4-5-20251001-v1:0".into(),
            name: "Claude Haiku 4.5 (US)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 5.0,
                cache_read_per_million: 0.1,
                cache_write_per_million: 1.25,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "us.anthropic.claude-opus-4-1-20250805-v1:0".into(),
            name: "Claude Opus 4.1 (US)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
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
            id: "us.anthropic.claude-opus-4-20250514-v1:0".into(),
            name: "Claude Opus 4 (US)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
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
            id: "us.anthropic.claude-opus-4-5-20251101-v1:0".into(),
            name: "Claude Opus 4.5 (US)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 25.0,
                cache_read_per_million: 0.5,
                cache_write_per_million: 6.25,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "us.anthropic.claude-opus-4-6-v1".into(),
            name: "Claude Opus 4.6 (US)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 128000,
            context_window: 1000000,
            cost: ModelCost {
                input_per_million: 5.0,
                output_per_million: 25.0,
                cache_read_per_million: 0.5,
                cache_write_per_million: 6.25,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "us.anthropic.claude-sonnet-4-20250514-v1:0".into(),
            name: "Claude Sonnet 4 (US)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
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
            id: "us.anthropic.claude-sonnet-4-5-20250929-v1:0".into(),
            name: "Claude Sonnet 4.5 (US)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
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
            id: "us.anthropic.claude-sonnet-4-6".into(),
            name: "Claude Sonnet 4.6 (US)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 1000000,
            cost: ModelCost {
                input_per_million: 3.0,
                output_per_million: 15.0,
                cache_read_per_million: 0.3,
                cache_write_per_million: 3.75,
            },
            headers: vec![],
            compat: None,
        },
        // ─── Bedrock / DeepSeek ───
        Model {
            id: "deepseek.r1-v1:0".into(),
            name: "DeepSeek-R1".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 32768,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 1.35,
                output_per_million: 5.4,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "deepseek.v3-v1:0".into(),
            name: "DeepSeek-V3.1".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 81920,
            context_window: 163840,
            cost: ModelCost {
                input_per_million: 0.58,
                output_per_million: 1.68,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "deepseek.v3.2".into(),
            name: "DeepSeek-V3.2".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 81920,
            context_window: 163840,
            cost: ModelCost {
                input_per_million: 0.62,
                output_per_million: 1.85,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ─── Bedrock / Google ───
        Model {
            id: "google.gemma-3-27b-it".into(),
            name: "Google Gemma 3 27B Instruct".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 8192,
            context_window: 202752,
            cost: ModelCost {
                input_per_million: 0.12,
                output_per_million: 0.2,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "google.gemma-3-4b-it".into(),
            name: "Gemma 3 4B IT".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.04,
                output_per_million: 0.08,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ─── Bedrock / Meta Llama ───
        Model {
            id: "meta.llama3-1-405b-instruct-v1:0".into(),
            name: "Llama 3.1 405B Instruct".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 2.4,
                output_per_million: 2.4,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "meta.llama3-1-70b-instruct-v1:0".into(),
            name: "Llama 3.1 70B Instruct".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.72,
                output_per_million: 0.72,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "meta.llama3-1-8b-instruct-v1:0".into(),
            name: "Llama 3.1 8B Instruct".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.22,
                output_per_million: 0.22,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "meta.llama3-2-11b-instruct-v1:0".into(),
            name: "Llama 3.2 11B Instruct".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.16,
                output_per_million: 0.16,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "meta.llama3-2-1b-instruct-v1:0".into(),
            name: "Llama 3.2 1B Instruct".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 131000,
            cost: ModelCost {
                input_per_million: 0.1,
                output_per_million: 0.1,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "meta.llama3-2-3b-instruct-v1:0".into(),
            name: "Llama 3.2 3B Instruct".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 131000,
            cost: ModelCost {
                input_per_million: 0.15,
                output_per_million: 0.15,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "meta.llama3-2-90b-instruct-v1:0".into(),
            name: "Llama 3.2 90B Instruct".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.72,
                output_per_million: 0.72,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "meta.llama3-3-70b-instruct-v1:0".into(),
            name: "Llama 3.3 70B Instruct".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.72,
                output_per_million: 0.72,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "meta.llama4-maverick-17b-instruct-v1:0".into(),
            name: "Llama 4 Maverick 17B Instruct".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16384,
            context_window: 1000000,
            cost: ModelCost {
                input_per_million: 0.24,
                output_per_million: 0.97,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "meta.llama4-scout-17b-instruct-v1:0".into(),
            name: "Llama 4 Scout 17B Instruct".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 16384,
            context_window: 3500000,
            cost: ModelCost {
                input_per_million: 0.17,
                output_per_million: 0.66,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ─── Bedrock / MiniMax ───
        Model {
            id: "minimax.minimax-m2".into(),
            name: "MiniMax M2".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 128000,
            context_window: 204608,
            cost: ModelCost {
                input_per_million: 0.3,
                output_per_million: 1.2,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "minimax.minimax-m2.1".into(),
            name: "MiniMax M2.1".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 131072,
            context_window: 204800,
            cost: ModelCost {
                input_per_million: 0.3,
                output_per_million: 1.2,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "minimax.minimax-m2.5".into(),
            name: "MiniMax M2.5".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 98304,
            context_window: 196608,
            cost: ModelCost {
                input_per_million: 0.3,
                output_per_million: 1.2,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ─── Bedrock / Mistral AI ───
        Model {
            id: "mistral.devstral-2-123b".into(),
            name: "Devstral 2 123B".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 256000,
            cost: ModelCost {
                input_per_million: 0.4,
                output_per_million: 2.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral.magistral-small-2509".into(),
            name: "Magistral Small 1.2".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 40000,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.5,
                output_per_million: 1.5,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral.ministral-3-14b-instruct".into(),
            name: "Ministral 14B 3.0".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.2,
                output_per_million: 0.2,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral.ministral-3-3b-instruct".into(),
            name: "Ministral 3 3B".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 8192,
            context_window: 256000,
            cost: ModelCost {
                input_per_million: 0.1,
                output_per_million: 0.1,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral.ministral-3-8b-instruct".into(),
            name: "Ministral 3 8B".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.15,
                output_per_million: 0.15,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral.mistral-large-3-675b-instruct".into(),
            name: "Mistral Large 3".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 8192,
            context_window: 256000,
            cost: ModelCost {
                input_per_million: 0.5,
                output_per_million: 1.5,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral.pixtral-large-2502-v1:0".into(),
            name: "Pixtral Large (25.02)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 8192,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 6.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral.voxtral-mini-3b-2507".into(),
            name: "Voxtral Mini 3B 2507".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.04,
                output_per_million: 0.04,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "mistral.voxtral-small-24b-2507".into(),
            name: "Voxtral Small 24B 2507".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 32000,
            cost: ModelCost {
                input_per_million: 0.15,
                output_per_million: 0.35,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ─── Bedrock / Moonshot (Kimi) ───
        Model {
            id: "moonshot.kimi-k2-thinking".into(),
            name: "Kimi K2 Thinking".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 256000,
            context_window: 256000,
            cost: ModelCost {
                input_per_million: 0.6,
                output_per_million: 2.5,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "moonshotai.kimi-k2.5".into(),
            name: "Kimi K2.5".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 256000,
            context_window: 256000,
            cost: ModelCost {
                input_per_million: 0.6,
                output_per_million: 3.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ─── Bedrock / NVIDIA ───
        Model {
            id: "nvidia.nemotron-nano-12b-v2".into(),
            name: "NVIDIA Nemotron Nano 12B v2 VL BF16".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.2,
                output_per_million: 0.6,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "nvidia.nemotron-nano-3-30b".into(),
            name: "NVIDIA Nemotron Nano 3 30B".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.06,
                output_per_million: 0.24,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "nvidia.nemotron-nano-9b-v2".into(),
            name: "NVIDIA Nemotron Nano 9B v2".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.06,
                output_per_million: 0.23,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "nvidia.nemotron-super-3-120b".into(),
            name: "NVIDIA Nemotron 3 Super 120B A12B".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 131072,
            context_window: 262144,
            cost: ModelCost {
                input_per_million: 0.15,
                output_per_million: 0.65,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ─── Bedrock / OpenAI (OSS) ───
        Model {
            id: "openai.gpt-oss-120b-1:0".into(),
            name: "gpt-oss-120b".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.15,
                output_per_million: 0.6,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "openai.gpt-oss-20b-1:0".into(),
            name: "gpt-oss-20b".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.07,
                output_per_million: 0.3,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "openai.gpt-oss-safeguard-120b".into(),
            name: "GPT OSS Safeguard 120B".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.15,
                output_per_million: 0.6,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "openai.gpt-oss-safeguard-20b".into(),
            name: "GPT OSS Safeguard 20B".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 0.07,
                output_per_million: 0.2,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ─── Bedrock / Qwen ───
        Model {
            id: "qwen.qwen3-235b-a22b-2507-v1:0".into(),
            name: "Qwen3 235B A22B 2507".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 131072,
            context_window: 262144,
            cost: ModelCost {
                input_per_million: 0.22,
                output_per_million: 0.88,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "qwen.qwen3-32b-v1:0".into(),
            name: "Qwen3 32B (dense)".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 16384,
            context_window: 16384,
            cost: ModelCost {
                input_per_million: 0.15,
                output_per_million: 0.6,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "qwen.qwen3-coder-30b-a3b-v1:0".into(),
            name: "Qwen3 Coder 30B A3B Instruct".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 131072,
            context_window: 262144,
            cost: ModelCost {
                input_per_million: 0.15,
                output_per_million: 0.6,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "qwen.qwen3-coder-480b-a35b-v1:0".into(),
            name: "Qwen3 Coder 480B A35B Instruct".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 65536,
            context_window: 131072,
            cost: ModelCost {
                input_per_million: 0.22,
                output_per_million: 1.8,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "qwen.qwen3-next-80b-a3b".into(),
            name: "Qwen/Qwen3-Next-80B-A3B-Instruct".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 262000,
            context_window: 262000,
            cost: ModelCost {
                input_per_million: 0.14,
                output_per_million: 1.4,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "qwen.qwen3-vl-235b-a22b".into(),
            name: "Qwen/Qwen3-VL-235B-A22B-Instruct".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 262000,
            context_window: 262000,
            cost: ModelCost {
                input_per_million: 0.3,
                output_per_million: 1.5,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ─── Bedrock / Writer ───
        Model {
            id: "writer.palmyra-x4-v1:0".into(),
            name: "Palmyra X4".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 122880,
            cost: ModelCost {
                input_per_million: 2.5,
                output_per_million: 10.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "writer.palmyra-x5-v1:0".into(),
            name: "Palmyra X5".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 1040000,
            cost: ModelCost {
                input_per_million: 0.6,
                output_per_million: 6.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ─── Bedrock / ZAI (Zhipu) ───
        Model {
            id: "zai.glm-4.7".into(),
            name: "GLM-4.7".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 131072,
            context_window: 204800,
            cost: ModelCost {
                input_per_million: 0.6,
                output_per_million: 2.2,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "zai.glm-4.7-flash".into(),
            name: "GLM-4.7-Flash".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 131072,
            context_window: 200000,
            cost: ModelCost {
                input_per_million: 0.07,
                output_per_million: 0.4,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "zai.glm-5".into(),
            name: "GLM-5".into(),
            api: api::BEDROCK_CONVERSE_STREAM.into(),
            provider: provider::AMAZON_BEDROCK.into(),
            base_url: String::new(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec![InputType::Text],
            max_tokens: 101376,
            context_window: 202752,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 3.2,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        // ── Google Vertex AI ──
        Model {
            id: "gemini-1.5-flash".into(),
            name: "Gemini 1.5 Flash (Vertex)".into(),
            api: api::GOOGLE_VERTEX.into(),
            provider: provider::GOOGLE_VERTEX.into(),
            base_url: String::new(),
            api_key_env: "GOOGLE_CLOUD_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 8192,
            context_window: 1000000,
            cost: ModelCost {
                input_per_million: 0.075,
                output_per_million: 0.3,
                cache_read_per_million: 0.01875,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gemini-1.5-flash-8b".into(),
            name: "Gemini 1.5 Flash-8B (Vertex)".into(),
            api: api::GOOGLE_VERTEX.into(),
            provider: provider::GOOGLE_VERTEX.into(),
            base_url: String::new(),
            api_key_env: "GOOGLE_CLOUD_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 8192,
            context_window: 1000000,
            cost: ModelCost {
                input_per_million: 0.0375,
                output_per_million: 0.15,
                cache_read_per_million: 0.01,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gemini-1.5-pro".into(),
            name: "Gemini 1.5 Pro (Vertex)".into(),
            api: api::GOOGLE_VERTEX.into(),
            provider: provider::GOOGLE_VERTEX.into(),
            base_url: String::new(),
            api_key_env: "GOOGLE_CLOUD_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 8192,
            context_window: 1000000,
            cost: ModelCost {
                input_per_million: 1.25,
                output_per_million: 5.0,
                cache_read_per_million: 0.3125,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gemini-2.0-flash".into(),
            name: "Gemini 2.0 Flash (Vertex)".into(),
            api: api::GOOGLE_VERTEX.into(),
            provider: provider::GOOGLE_VERTEX.into(),
            base_url: String::new(),
            api_key_env: "GOOGLE_CLOUD_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 8192,
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
            id: "gemini-2.0-flash-lite".into(),
            name: "Gemini 2.0 Flash Lite (Vertex)".into(),
            api: api::GOOGLE_VERTEX.into(),
            provider: provider::GOOGLE_VERTEX.into(),
            base_url: String::new(),
            api_key_env: "GOOGLE_CLOUD_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 65536,
            context_window: 1048576,
            cost: ModelCost {
                input_per_million: 0.075,
                output_per_million: 0.3,
                cache_read_per_million: 0.01875,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gemini-2.5-flash".into(),
            name: "Gemini 2.5 Flash (Vertex)".into(),
            api: api::GOOGLE_VERTEX.into(),
            provider: provider::GOOGLE_VERTEX.into(),
            base_url: String::new(),
            api_key_env: "GOOGLE_CLOUD_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 65536,
            context_window: 1048576,
            cost: ModelCost {
                input_per_million: 0.3,
                output_per_million: 2.5,
                cache_read_per_million: 0.03,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gemini-2.5-flash-lite".into(),
            name: "Gemini 2.5 Flash Lite (Vertex)".into(),
            api: api::GOOGLE_VERTEX.into(),
            provider: provider::GOOGLE_VERTEX.into(),
            base_url: String::new(),
            api_key_env: "GOOGLE_CLOUD_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 65536,
            context_window: 1048576,
            cost: ModelCost {
                input_per_million: 0.1,
                output_per_million: 0.4,
                cache_read_per_million: 0.01,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gemini-2.5-flash-lite-preview-09-2025".into(),
            name: "Gemini 2.5 Flash Lite Preview 09-25 (Vertex)".into(),
            api: api::GOOGLE_VERTEX.into(),
            provider: provider::GOOGLE_VERTEX.into(),
            base_url: String::new(),
            api_key_env: "GOOGLE_CLOUD_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 65536,
            context_window: 1048576,
            cost: ModelCost {
                input_per_million: 0.1,
                output_per_million: 0.4,
                cache_read_per_million: 0.01,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gemini-2.5-pro".into(),
            name: "Gemini 2.5 Pro (Vertex)".into(),
            api: api::GOOGLE_VERTEX.into(),
            provider: provider::GOOGLE_VERTEX.into(),
            base_url: String::new(),
            api_key_env: "GOOGLE_CLOUD_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 65536,
            context_window: 1048576,
            cost: ModelCost {
                input_per_million: 1.25,
                output_per_million: 10.0,
                cache_read_per_million: 0.125,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gemini-3-flash-preview".into(),
            name: "Gemini 3 Flash Preview (Vertex)".into(),
            api: api::GOOGLE_VERTEX.into(),
            provider: provider::GOOGLE_VERTEX.into(),
            base_url: String::new(),
            api_key_env: "GOOGLE_CLOUD_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 65536,
            context_window: 1048576,
            cost: ModelCost {
                input_per_million: 0.5,
                output_per_million: 3.0,
                cache_read_per_million: 0.05,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gemini-3-pro-preview".into(),
            name: "Gemini 3 Pro Preview (Vertex)".into(),
            api: api::GOOGLE_VERTEX.into(),
            provider: provider::GOOGLE_VERTEX.into(),
            base_url: String::new(),
            api_key_env: "GOOGLE_CLOUD_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 64000,
            context_window: 1000000,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 12.0,
                cache_read_per_million: 0.2,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
        Model {
            id: "gemini-3.1-pro-preview".into(),
            name: "Gemini 3.1 Pro Preview (Vertex)".into(),
            api: api::GOOGLE_VERTEX.into(),
            provider: provider::GOOGLE_VERTEX.into(),
            base_url: String::new(),
            api_key_env: "GOOGLE_CLOUD_API_KEY".into(),
            reasoning: true,
            input: vec![InputType::Text, InputType::Image],
            max_tokens: 65536,
            context_window: 1048576,
            cost: ModelCost {
                input_per_million: 2.0,
                output_per_million: 12.0,
                cache_read_per_million: 0.2,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        },
    ]
});

fn copilot_headers() -> Vec<(String, String)> {
    vec![
        ("User-Agent".into(), "GitHubCopilotChat/0.35.0".into()),
        ("Editor-Version".into(), "vscode/1.107.0".into()),
        ("Editor-Plugin-Version".into(), "copilot-chat/0.35.0".into()),
        ("Copilot-Integration-Id".into(), "vscode-chat".into()),
    ]
}

/// Compat flags for GitHub Copilot openai-completions models.
/// Pi-mono sets supportsStore=false, supportsDeveloperRole=false,
/// supportsReasoningEffort=false for these models.
fn copilot_completions_compat() -> ProviderCompat {
    ProviderCompat {
        supports_store: false,
        supports_developer_role: false,
        supports_reasoning_effort: false,
        ..ProviderCompat::default()
    }
}

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
        "groq",
        "xai",
        "cerebras",
        "qwen",
        "doubao",
        "kimi",
        "minimax",
        "zai",
        "deepseek",
        "mistral",
        "github-copilot",
        "azure-openai-responses",
        "amazon-bedrock",
        "google-vertex",
    ]
}

// ============================================================================
// Model auto-discovery — /v1/models endpoint probe
// ============================================================================

/// A model discovered from a remote `/v1/models` endpoint.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredModel {
    pub id: String,
    #[serde(default)]
    pub owned_by: String,
    #[serde(default)]
    pub created: i64,
}

/// Errors from model discovery.
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API returned error {status}: {body}")]
    ApiError { status: u16, body: String },
    #[error("failed to parse response: {0}")]
    Parse(String),
}

/// Query an OpenAI-compatible `/v1/models` endpoint and return available models.
///
/// `base_url` should be the API root (e.g., `http://localhost:11434/v1` for Ollama,
/// or `https://api.openai.com/v1`). The function appends `/models` automatically.
///
/// If `api_key` is `None`, the request is sent without an Authorization header
/// (common for local inference servers).
pub async fn discover_models(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<DiscoveredModel>, DiscoveryError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    discover_models_with_client(&client, base_url, api_key).await
}

/// Like [`discover_models`], but uses a caller-supplied HTTP client.
///
/// Useful when you need to control proxy settings, timeouts, or connection pooling.
pub async fn discover_models_with_client(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<DiscoveredModel>, DiscoveryError> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));

    let mut req = client.get(&url);
    if let Some(key) = api_key {
        req = req.header("Authorization", format!("Bearer {key}"));
    }

    let response = req.send().await?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        return Err(DiscoveryError::ApiError { status, body });
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| DiscoveryError::Parse(e.to_string()))?;

    let data = body
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| DiscoveryError::Parse("missing 'data' array in response".into()))?;

    let models: Vec<DiscoveredModel> = data
        .iter()
        .filter_map(|v| {
            serde_json::from_value::<DiscoveredModel>(v.clone())
                .map_err(|e| tracing::debug!("skipping malformed model entry: {e}"))
                .ok()
        })
        .collect();

    Ok(models)
}

/// Construct a usable `Model` from a `DiscoveredModel` with conservative defaults.
///
/// This creates a Model configured for the `openai-completions` API type, which works
/// with any OpenAI-compatible endpoint.
///
/// **Important**: `max_tokens` (4096) and `context_window` (8192) are conservative
/// estimates. Callers should override these based on the actual model's capabilities.
/// Some endpoints (vLLM: `max_model_len`, LM Studio: `context_length`) return this
/// info in the `/v1/models` response — future versions may extract it automatically.
pub fn construct_model_from_discovered(
    discovered: &DiscoveredModel,
    provider_name: &str,
    base_url: &str,
    api_key_env: &str,
) -> Model {
    Model {
        id: discovered.id.clone(),
        name: discovered.id.clone(),
        api: api::OPENAI_COMPLETIONS.into(),
        provider: provider_name.into(),
        base_url: base_url.into(),
        api_key_env: api_key_env.into(),
        reasoning: false,
        input: vec![InputType::Text],
        max_tokens: 4096,
        context_window: 8192,
        cost: ModelCost {
            input_per_million: 0.0,
            output_per_million: 0.0,
            cache_read_per_million: 0.0,
            cache_write_per_million: 0.0,
        },
        headers: vec![],
        compat: default_compat(),
    }
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
    fn test_resolve_qwen_3_6_plus() {
        let model = resolve_model("qwen", "qwen3.6-plus").unwrap();
        assert_eq!(model.id, "qwen3.6-plus");
        assert_eq!(model.provider, "qwen");
        assert_eq!(model.api_key_env, "DASHSCOPE_API_KEY");
        assert_eq!(
            model.base_url,
            "https://dashscope.aliyuncs.com/compatible-mode/v1"
        );
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
    fn test_resolve_kimi_k2_5() {
        let model = resolve_model("kimi", "kimi-k2.5").unwrap();
        assert_eq!(model.id, "kimi-k2.5");
        assert_eq!(model.provider, "kimi");
        assert_eq!(model.api_key_env, "MOONSHOT_API_KEY");
        assert_eq!(model.base_url, "https://api.moonshot.cn/v1");
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
    fn test_list_providers_contains_all() {
        let providers = list_providers();
        let expected = [
            "anthropic",
            "openai",
            "google",
            "groq",
            "xai",
            "cerebras",
            "qwen",
            "doubao",
            "kimi",
            "minimax",
            "zai",
            "deepseek",
            "mistral",
            "github-copilot",
            "azure-openai-responses",
            "amazon-bedrock",
            "google-vertex",
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
    fn test_list_models_no_duplicate_provider_id_pairs() {
        let models = list_models();
        let mut seen = std::collections::HashSet::new();
        for model in models {
            let key = format!("{}:{}", model.provider, model.id);
            assert!(
                seen.insert(key.clone()),
                "duplicate (provider, id) pair found: {}",
                key
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

    // ========================================================================
    // resolve_model — Groq models
    // ========================================================================

    #[test]
    fn test_resolve_groq_qwen3_32b() {
        let model = resolve_model("groq", "qwen/qwen3-32b").unwrap();
        assert_eq!(model.id, "qwen/qwen3-32b");
        assert_eq!(model.name, "Qwen3 32B");
        assert_eq!(model.provider, "groq");
        assert_eq!(model.api, api::OPENAI_COMPLETIONS);
        assert_eq!(model.api_key_env, "GROQ_API_KEY");
        assert_eq!(model.base_url, "https://api.groq.com/openai/v1");
        assert!(model.reasoning);
        assert_eq!(model.max_tokens, 16384);
        assert_eq!(model.context_window, 131072);
        // compat: None — detect_compat handles Groq dynamically
        assert!(model.compat.is_none());
    }

    #[test]
    fn test_resolve_groq_llama33_70b() {
        let model = resolve_model("groq", "llama-3.3-70b-versatile").unwrap();
        assert_eq!(model.id, "llama-3.3-70b-versatile");
        assert!(!model.reasoning);
        assert_eq!(model.max_tokens, 32768);
        assert_eq!(model.context_window, 131072);
    }

    #[test]
    fn test_resolve_groq_llama4_maverick() {
        let model = resolve_model("groq", "meta-llama/llama-4-maverick-17b-128e-instruct").unwrap();
        assert_eq!(model.name, "Llama 4 Maverick 17B");
        assert!(!model.reasoning);
        assert!(model.input.contains(&InputType::Image));
    }

    #[test]
    fn test_resolve_groq_deepseek_r1() {
        let model = resolve_model("groq", "deepseek-r1-distill-llama-70b").unwrap();
        assert!(model.reasoning);
        assert_eq!(model.cost.input_per_million, 0.75);
    }

    #[test]
    fn test_resolve_groq_qwq_32b() {
        let model = resolve_model("groq", "qwen-qwq-32b").unwrap();
        assert!(model.reasoning);
        assert_eq!(model.max_tokens, 16384);
    }

    #[test]
    fn test_resolve_groq_gpt_oss_120b() {
        let model = resolve_model("groq", "openai/gpt-oss-120b").unwrap();
        assert!(model.reasoning);
        assert_eq!(model.max_tokens, 65536);
    }

    #[test]
    fn test_groq_in_list_providers() {
        let providers = list_providers();
        assert!(providers.contains(&"groq"));
    }

    #[test]
    fn test_groq_model_count() {
        let groq_models: Vec<_> = list_models()
            .iter()
            .filter(|m| m.provider == "groq")
            .collect();
        assert_eq!(groq_models.len(), 15);
    }

    #[test]
    fn test_groq_models_invariants() {
        for m in list_models().iter().filter(|m| m.provider == "groq") {
            assert_eq!(
                m.base_url, "https://api.groq.com/openai/v1",
                "bad base_url for {}",
                m.id
            );
            assert_eq!(
                m.api_key_env, "GROQ_API_KEY",
                "bad api_key_env for {}",
                m.id
            );
            assert_eq!(m.api, api::OPENAI_COMPLETIONS, "bad api for {}", m.id);
            assert!(m.compat.is_none(), "unexpected compat for {}", m.id);
        }
    }

    // ========================================================================
    // resolve_model — xAI (Grok) models
    // ========================================================================

    #[test]
    fn test_resolve_xai_grok4() {
        let model = resolve_model("xai", "grok-4").unwrap();
        assert_eq!(model.id, "grok-4");
        assert_eq!(model.name, "Grok 4");
        assert_eq!(model.provider, "xai");
        assert_eq!(model.api, api::OPENAI_COMPLETIONS);
        assert_eq!(model.api_key_env, "XAI_API_KEY");
        assert_eq!(model.base_url, "https://api.x.ai/v1");
        assert!(model.reasoning);
        assert_eq!(model.max_tokens, 64000);
        assert_eq!(model.context_window, 256000);
        assert!(model.compat.is_none());
    }

    #[test]
    fn test_resolve_xai_grok4_fast() {
        let model = resolve_model("xai", "grok-4-fast").unwrap();
        assert_eq!(model.name, "Grok 4 Fast");
        assert!(model.reasoning);
        assert!(model.input.contains(&InputType::Image));
        assert_eq!(model.max_tokens, 30000);
        assert_eq!(model.context_window, 2000000);
    }

    #[test]
    fn test_resolve_xai_grok3_mini() {
        let model = resolve_model("xai", "grok-3-mini").unwrap();
        assert!(model.reasoning);
        assert_eq!(model.cost.input_per_million, 0.3);
        assert_eq!(model.cost.cache_read_per_million, 0.075);
    }

    #[test]
    fn test_resolve_xai_grok2_vision() {
        let model = resolve_model("xai", "grok-2-vision").unwrap();
        assert!(!model.reasoning);
        assert!(model.input.contains(&InputType::Image));
        assert_eq!(model.context_window, 8192);
    }

    #[test]
    fn test_resolve_xai_grok4_non_reasoning() {
        let model = resolve_model("xai", "grok-4-fast-non-reasoning").unwrap();
        assert!(!model.reasoning);
        assert!(model.input.contains(&InputType::Image));
        assert_eq!(model.context_window, 2000000);
    }

    #[test]
    fn test_resolve_xai_grok_code_fast() {
        let model = resolve_model("xai", "grok-code-fast-1").unwrap();
        assert!(model.reasoning);
        assert_eq!(model.max_tokens, 10000);
        assert_eq!(model.context_window, 256000);
        assert_eq!(model.cost.cache_read_per_million, 0.02);
    }

    #[test]
    fn test_xai_in_list_providers() {
        let providers = list_providers();
        assert!(providers.contains(&"xai"));
    }

    #[test]
    fn test_xai_model_count() {
        let xai_models: Vec<_> = list_models()
            .iter()
            .filter(|m| m.provider == "xai")
            .collect();
        assert_eq!(xai_models.len(), 24);
    }

    #[test]
    fn test_xai_models_invariants() {
        for m in list_models().iter().filter(|m| m.provider == "xai") {
            assert_eq!(
                m.base_url, "https://api.x.ai/v1",
                "bad base_url for {}",
                m.id
            );
            assert_eq!(m.api_key_env, "XAI_API_KEY", "bad api_key_env for {}", m.id);
            assert_eq!(m.api, api::OPENAI_COMPLETIONS, "bad api for {}", m.id);
            assert!(m.compat.is_none(), "unexpected compat for {}", m.id);
        }
    }

    // ========================================================================
    // resolve_model — Cerebras models
    // ========================================================================

    #[test]
    fn test_resolve_cerebras_gpt_oss_120b() {
        let model = resolve_model("cerebras", "gpt-oss-120b").unwrap();
        assert_eq!(model.id, "gpt-oss-120b");
        assert_eq!(model.name, "GPT OSS 120B");
        assert_eq!(model.provider, "cerebras");
        assert_eq!(model.api, api::OPENAI_COMPLETIONS);
        assert_eq!(model.base_url, "https://api.cerebras.ai/v1");
        assert!(model.reasoning);
        assert_eq!(model.max_tokens, 32768);
        assert_eq!(model.context_window, 131072);
    }

    #[test]
    fn test_resolve_cerebras_llama31_8b() {
        let model = resolve_model("cerebras", "llama3.1-8b").unwrap();
        assert!(!model.reasoning);
        assert_eq!(model.max_tokens, 8000);
        assert_eq!(model.context_window, 32000);
    }

    #[test]
    fn test_resolve_cerebras_zai_glm() {
        let model = resolve_model("cerebras", "zai-glm-4.7").unwrap();
        assert_eq!(model.cost.input_per_million, 2.25);
        assert_eq!(model.cost.output_per_million, 2.75);
    }

    #[test]
    fn test_cerebras_in_list_providers() {
        let providers = list_providers();
        assert!(providers.contains(&"cerebras"));
    }

    #[test]
    fn test_cerebras_model_count() {
        let models: Vec<_> = list_models()
            .iter()
            .filter(|m| m.provider == "cerebras")
            .collect();
        assert_eq!(models.len(), 4);
    }

    #[test]
    fn test_cerebras_models_invariants() {
        for m in list_models().iter().filter(|m| m.provider == "cerebras") {
            assert_eq!(
                m.base_url, "https://api.cerebras.ai/v1",
                "bad base_url for {}",
                m.id
            );
            assert_eq!(
                m.api_key_env, "CEREBRAS_API_KEY",
                "bad api_key_env for {}",
                m.id
            );
            assert_eq!(m.api, api::OPENAI_COMPLETIONS, "bad api for {}", m.id);
            assert!(m.compat.is_none(), "unexpected compat for {}", m.id);
        }
    }

    // ========================================================================
    // resolve_model — Mistral models
    // ========================================================================

    #[test]
    fn test_resolve_mistral_codestral_latest() {
        let model = resolve_model("mistral", "codestral-latest").unwrap();
        assert_eq!(model.id, "codestral-latest");
        assert_eq!(model.name, "Codestral (latest)");
        assert_eq!(model.provider, "mistral");
        assert_eq!(model.api, api::OPENAI_COMPLETIONS);
        assert_eq!(model.base_url, "https://api.mistral.ai/v1");
        assert_eq!(model.api_key_env, "MISTRAL_API_KEY");
        assert!(!model.reasoning);
        assert_eq!(model.context_window, 256000);
    }

    #[test]
    fn test_resolve_mistral_magistral_medium() {
        let model = resolve_model("mistral", "magistral-medium-latest").unwrap();
        assert_eq!(model.name, "Magistral Medium (latest)");
        assert!(model.reasoning);
        assert_eq!(model.cost.input_per_million, 2.0);
        assert_eq!(model.cost.output_per_million, 5.0);
    }

    #[test]
    fn test_resolve_mistral_magistral_small() {
        let model = resolve_model("mistral", "magistral-small").unwrap();
        assert!(model.reasoning);
        assert_eq!(model.max_tokens, 128000);
    }

    #[test]
    fn test_resolve_mistral_large_latest() {
        let model = resolve_model("mistral", "mistral-large-latest").unwrap();
        assert!(!model.reasoning);
        assert!(model.input.contains(&InputType::Image));
        assert_eq!(model.context_window, 262144);
    }

    #[test]
    fn test_resolve_mistral_pixtral_12b() {
        let model = resolve_model("mistral", "pixtral-12b").unwrap();
        assert!(model.input.contains(&InputType::Image));
        assert_eq!(model.cost.input_per_million, 0.15);
    }

    #[test]
    fn test_resolve_mistral_nemo() {
        let model = resolve_model("mistral", "mistral-nemo").unwrap();
        assert!(!model.reasoning);
        assert_eq!(model.cost.input_per_million, 0.15);
        assert_eq!(model.cost.output_per_million, 0.15);
    }

    #[test]
    fn test_resolve_mistral_open_mixtral_8x22b() {
        let model = resolve_model("mistral", "open-mixtral-8x22b").unwrap();
        assert_eq!(model.name, "Mixtral 8x22B");
        assert_eq!(model.cost.input_per_million, 2.0);
        assert_eq!(model.context_window, 64000);
    }

    #[test]
    fn test_mistral_in_list_providers() {
        let providers = list_providers();
        assert!(providers.contains(&"mistral"));
    }

    #[test]
    fn test_mistral_model_count() {
        let models: Vec<_> = list_models()
            .iter()
            .filter(|m| m.provider == "mistral")
            .collect();
        assert_eq!(models.len(), 25);
    }

    #[test]
    fn test_mistral_models_invariants() {
        for m in list_models().iter().filter(|m| m.provider == "mistral") {
            assert_eq!(
                m.base_url, "https://api.mistral.ai/v1",
                "bad base_url for {}",
                m.id
            );
            assert_eq!(
                m.api_key_env, "MISTRAL_API_KEY",
                "bad api_key_env for {}",
                m.id
            );
            assert_eq!(m.api, api::OPENAI_COMPLETIONS, "bad api for {}", m.id);
            assert!(m.compat.is_none(), "unexpected compat for {}", m.id);
        }
    }

    // ========================================================================
    // resolve_model — GitHub Copilot models
    // ========================================================================

    #[test]
    fn test_resolve_copilot_claude_opus46() {
        let model = resolve_model("github-copilot", "claude-opus-4.6").unwrap();
        assert_eq!(model.id, "claude-opus-4.6");
        assert_eq!(model.name, "Claude Opus 4.6");
        assert_eq!(model.provider, "github-copilot");
        assert_eq!(model.api, api::ANTHROPIC_MESSAGES);
        assert_eq!(model.base_url, "https://api.individual.githubcopilot.com");
        assert_eq!(model.api_key_env, "COPILOT_GITHUB_TOKEN");
        assert!(model.reasoning);
        assert!(model.input.contains(&InputType::Image));
        assert_eq!(model.max_tokens, 64000);
        assert_eq!(model.context_window, 1000000);
    }

    #[test]
    fn test_resolve_copilot_gpt5() {
        let model = resolve_model("github-copilot", "gpt-5").unwrap();
        assert_eq!(model.api, api::OPENAI_RESPONSES);
        assert!(model.reasoning);
        assert_eq!(model.max_tokens, 128000);
    }

    #[test]
    fn test_resolve_copilot_gpt51_codex() {
        let model = resolve_model("github-copilot", "gpt-5.1-codex").unwrap();
        assert_eq!(model.api, api::OPENAI_RESPONSES);
        assert_eq!(model.context_window, 400000);
    }

    #[test]
    fn test_resolve_copilot_gemini_25_pro() {
        let model = resolve_model("github-copilot", "gemini-2.5-pro").unwrap();
        assert_eq!(model.api, api::OPENAI_COMPLETIONS);
        assert!(!model.reasoning);
    }

    #[test]
    fn test_resolve_copilot_gpt4o() {
        let model = resolve_model("github-copilot", "gpt-4o").unwrap();
        assert_eq!(model.api, api::OPENAI_COMPLETIONS);
        assert!(!model.reasoning);
        assert_eq!(model.max_tokens, 4096);
    }

    #[test]
    fn test_resolve_copilot_grok_code_fast() {
        let model = resolve_model("github-copilot", "grok-code-fast-1").unwrap();
        assert_eq!(model.api, api::OPENAI_COMPLETIONS);
        assert!(model.reasoning);
        // grok-code-fast-1 is text-only on Copilot
        assert!(!model.input.contains(&InputType::Image));
    }

    #[test]
    fn test_copilot_in_list_providers() {
        let providers = list_providers();
        assert!(providers.contains(&"github-copilot"));
    }

    #[test]
    fn test_copilot_model_count() {
        let models: Vec<_> = list_models()
            .iter()
            .filter(|m| m.provider == "github-copilot")
            .collect();
        assert_eq!(models.len(), 24);
    }

    #[test]
    fn test_copilot_models_invariants() {
        for m in list_models()
            .iter()
            .filter(|m| m.provider == "github-copilot")
        {
            assert_eq!(
                m.base_url, "https://api.individual.githubcopilot.com",
                "bad base_url for {}",
                m.id
            );
            assert_eq!(
                m.api_key_env, "COPILOT_GITHUB_TOKEN",
                "bad api_key_env for {}",
                m.id
            );
            // openai-completions models have explicit compat; others have None
            if m.api == api::OPENAI_COMPLETIONS {
                assert!(
                    m.compat.is_some(),
                    "completions model {} must have compat",
                    m.id
                );
            } else {
                assert!(
                    m.compat.is_none(),
                    "non-completions model {} should have compat: None",
                    m.id
                );
            }
            // All copilot models have zero cost
            assert_eq!(m.cost.input_per_million, 0.0, "bad cost for {}", m.id);
            assert_eq!(m.cost.output_per_million, 0.0, "bad cost for {}", m.id);
            // All copilot models carry static headers
            assert!(!m.headers.is_empty(), "missing headers for {}", m.id);
            assert!(
                m.headers.iter().any(|(k, _)| k == "Copilot-Integration-Id"),
                "missing Copilot-Integration-Id header for {}",
                m.id
            );
        }
    }

    #[test]
    fn test_copilot_api_types_distribution() {
        let copilot: Vec<_> = list_models()
            .iter()
            .filter(|m| m.provider == "github-copilot")
            .cloned()
            .collect();
        let anthropic_count = copilot
            .iter()
            .filter(|m| m.api == api::ANTHROPIC_MESSAGES)
            .count();
        let completions_count = copilot
            .iter()
            .filter(|m| m.api == api::OPENAI_COMPLETIONS)
            .count();
        let responses_count = copilot
            .iter()
            .filter(|m| m.api == api::OPENAI_RESPONSES)
            .count();
        assert_eq!(anthropic_count, 6, "expected 6 anthropic-messages models");
        assert_eq!(completions_count, 7, "expected 7 openai-completions models");
        assert_eq!(responses_count, 11, "expected 11 openai-responses models");
    }

    #[test]
    fn test_copilot_completions_models_have_compat() {
        let completions: Vec<_> = list_models()
            .iter()
            .filter(|m| m.provider == "github-copilot" && m.api == api::OPENAI_COMPLETIONS)
            .cloned()
            .collect();
        assert_eq!(completions.len(), 7);
        for m in &completions {
            let compat = m.compat.as_ref().unwrap_or_else(|| {
                panic!(
                    "copilot openai-completions model '{}' must have compat",
                    m.id
                )
            });
            assert!(
                !compat.supports_store,
                "{}: supports_store should be false",
                m.id
            );
            assert!(
                !compat.supports_developer_role,
                "{}: supports_developer_role should be false",
                m.id
            );
            assert!(
                !compat.supports_reasoning_effort,
                "{}: supports_reasoning_effort should be false",
                m.id
            );
        }
    }

    #[test]
    fn test_copilot_non_completions_models_no_compat() {
        let non_completions: Vec<_> = list_models()
            .iter()
            .filter(|m| m.provider == "github-copilot" && m.api != api::OPENAI_COMPLETIONS)
            .cloned()
            .collect();
        assert_eq!(non_completions.len(), 17); // 6 anthropic + 11 responses
        for m in &non_completions {
            assert!(
                m.compat.is_none(),
                "copilot non-completions model '{}' should have compat: None",
                m.id
            );
        }
    }

    // ========================================================================
    // Azure OpenAI Responses model catalog
    // ========================================================================

    #[test]
    fn test_azure_models_count() {
        let models: Vec<_> = list_models()
            .iter()
            .filter(|m| m.provider == "azure-openai-responses")
            .collect();
        assert_eq!(
            models.len(),
            40,
            "expected 40 azure-openai-responses models"
        );
    }

    #[test]
    fn test_azure_models_invariants() {
        for m in list_models()
            .iter()
            .filter(|m| m.provider == "azure-openai-responses")
        {
            assert_eq!(m.api, api::AZURE_OPENAI_RESPONSES, "bad api for {}", m.id);
            assert!(
                m.base_url.is_empty(),
                "azure base_url should be empty for {} (resolved at runtime)",
                m.id
            );
            assert_eq!(
                m.api_key_env, "AZURE_OPENAI_API_KEY",
                "bad api_key_env for {}",
                m.id
            );
            assert!(
                m.compat.is_none(),
                "azure model {} should have compat: None",
                m.id
            );
            assert!(
                m.headers.is_empty(),
                "azure model {} should have no static headers",
                m.id
            );
            assert!(
                m.max_tokens <= m.context_window,
                "max_tokens > context_window for {}",
                m.id
            );
        }
    }

    #[test]
    fn test_azure_models_reasoning_flags() {
        let azure: Vec<_> = list_models()
            .iter()
            .filter(|m| m.provider == "azure-openai-responses")
            .cloned()
            .collect();
        // o-series and gpt-5 reasoning models should have reasoning=true
        for m in &azure {
            if m.id.starts_with("o1") || m.id.starts_with("o3") || m.id.starts_with("o4") {
                assert!(
                    m.reasoning,
                    "o-series model {} should have reasoning=true",
                    m.id
                );
            }
        }
        // gpt-4 (non-turbo) should have reasoning=false
        let gpt4 = azure.iter().find(|m| m.id == "gpt-4").unwrap();
        assert!(!gpt4.reasoning, "gpt-4 should not be a reasoning model");
    }

    #[test]
    fn test_azure_resolve_gpt4o() {
        let m = resolve_model("azure-openai-responses", "gpt-4o").unwrap();
        assert_eq!(m.api, api::AZURE_OPENAI_RESPONSES);
        assert!(!m.reasoning);
        assert!(m.input.contains(&InputType::Image));
        assert_eq!(m.context_window, 128000);
    }

    #[test]
    fn test_azure_resolve_o3() {
        let m = resolve_model("azure-openai-responses", "o3").unwrap();
        assert_eq!(m.api, api::AZURE_OPENAI_RESPONSES);
        assert!(m.reasoning);
        assert_eq!(m.context_window, 200000);
    }

    // ========================================================================
    // Amazon Bedrock model catalog
    // ========================================================================

    #[test]
    fn test_bedrock_models_count() {
        let models: Vec<_> = list_models()
            .iter()
            .filter(|m| m.provider == "amazon-bedrock")
            .collect();
        assert_eq!(models.len(), 86, "expected 86 amazon-bedrock models");
    }

    #[test]
    fn test_bedrock_models_invariants() {
        for m in list_models()
            .iter()
            .filter(|m| m.provider == "amazon-bedrock")
        {
            assert_eq!(m.api, api::BEDROCK_CONVERSE_STREAM, "bad api for {}", m.id);
            assert!(
                m.base_url.is_empty(),
                "bedrock base_url should be empty for {} (resolved via AWS SDK)",
                m.id
            );
            assert_eq!(
                m.api_key_env, "AWS_ACCESS_KEY_ID",
                "bad api_key_env for {}",
                m.id
            );
            assert!(
                m.compat.is_none(),
                "bedrock model {} should have compat: None",
                m.id
            );
            assert!(
                m.headers.is_empty(),
                "bedrock model {} should have no static headers",
                m.id
            );
            assert!(
                m.max_tokens <= m.context_window,
                "max_tokens > context_window for {}",
                m.id
            );
        }
    }

    #[test]
    fn test_bedrock_models_reasoning_flags() {
        let bedrock: Vec<_> = list_models()
            .iter()
            .filter(|m| m.provider == "amazon-bedrock")
            .cloned()
            .collect();
        // Claude 4.x models should have reasoning=true
        for m in &bedrock {
            if m.id.contains("claude-opus-4") || m.id.contains("claude-sonnet-4") {
                assert!(
                    m.reasoning,
                    "Claude 4.x model {} should have reasoning=true",
                    m.id
                );
            }
        }
        // Nova Lite should not have reasoning
        let nova_lite = bedrock
            .iter()
            .find(|m| m.id == "amazon.nova-lite-v1:0")
            .unwrap();
        assert!(
            !nova_lite.reasoning,
            "Nova Lite should not be a reasoning model"
        );
    }

    #[test]
    fn test_bedrock_resolve_claude_opus() {
        let m = resolve_model("amazon-bedrock", "anthropic.claude-opus-4-6-v1").unwrap();
        assert_eq!(m.api, api::BEDROCK_CONVERSE_STREAM);
        assert!(m.reasoning);
        assert!(m.input.contains(&InputType::Image));
    }

    #[test]
    fn test_bedrock_resolve_nova_pro() {
        let m = resolve_model("amazon-bedrock", "amazon.nova-pro-v1:0").unwrap();
        assert_eq!(m.api, api::BEDROCK_CONVERSE_STREAM);
        assert!(!m.reasoning);
        assert!(m.input.contains(&InputType::Image));
    }

    // ========================================================================
    // DiscoveredModel — serde
    // ========================================================================

    #[test]
    fn test_discovered_model_deserialize_full() {
        let json = r#"{"id": "llama3.2:latest", "owned_by": "library", "created": 1700000000}"#;
        let m: DiscoveredModel = serde_json::from_str(json).unwrap();
        assert_eq!(m.id, "llama3.2:latest");
        assert_eq!(m.owned_by, "library");
        assert_eq!(m.created, 1700000000);
    }

    #[test]
    fn test_discovered_model_deserialize_minimal() {
        // Some endpoints only return id
        let json = r#"{"id": "qwen2.5-coder:7b"}"#;
        let m: DiscoveredModel = serde_json::from_str(json).unwrap();
        assert_eq!(m.id, "qwen2.5-coder:7b");
        assert_eq!(m.owned_by, ""); // default
        assert_eq!(m.created, 0); // default
    }

    #[test]
    fn test_discovered_model_deserialize_extra_fields_ignored() {
        let json = r#"{"id": "model-x", "owned_by": "me", "created": 0, "object": "model", "permission": []}"#;
        let m: DiscoveredModel = serde_json::from_str(json).unwrap();
        assert_eq!(m.id, "model-x");
    }

    #[test]
    fn test_discovered_model_serialize_roundtrip() {
        let m = DiscoveredModel {
            id: "gpt-4o".into(),
            owned_by: "openai".into(),
            created: 1700000000,
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: DiscoveredModel = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "gpt-4o");
        assert_eq!(back.owned_by, "openai");
    }

    // ========================================================================
    // construct_model_from_discovered
    // ========================================================================

    #[test]
    fn test_construct_model_uses_discovered_id() {
        let d = DiscoveredModel {
            id: "llama3.2:latest".into(),
            owned_by: "library".into(),
            created: 0,
        };
        let model = construct_model_from_discovered(
            &d,
            "ollama",
            "http://localhost:11434/v1",
            "OLLAMA_API_KEY",
        );
        assert_eq!(model.id, "llama3.2:latest");
        assert_eq!(model.name, "llama3.2:latest");
        assert_eq!(model.provider, "ollama");
        assert_eq!(model.base_url, "http://localhost:11434/v1");
        assert_eq!(model.api_key_env, "OLLAMA_API_KEY");
    }

    #[test]
    fn test_construct_model_defaults_to_openai_completions_api() {
        let d = DiscoveredModel {
            id: "some-model".into(),
            owned_by: String::new(),
            created: 0,
        };
        let model = construct_model_from_discovered(&d, "local", "http://localhost:8080/v1", "");
        assert_eq!(model.api, api::OPENAI_COMPLETIONS);
    }

    #[test]
    fn test_construct_model_has_sensible_defaults() {
        let d = DiscoveredModel {
            id: "test".into(),
            owned_by: String::new(),
            created: 0,
        };
        let model = construct_model_from_discovered(&d, "test", "http://localhost/v1", "");
        assert_eq!(model.max_tokens, 4096);
        assert_eq!(model.context_window, 8192);
        assert!(!model.reasoning);
        assert!(model.input.contains(&InputType::Text));
        assert_eq!(model.cost.input_per_million, 0.0);
    }

    #[test]
    fn test_construct_model_has_compat() {
        let d = DiscoveredModel {
            id: "test".into(),
            owned_by: String::new(),
            created: 0,
        };
        let model = construct_model_from_discovered(&d, "test", "http://localhost/v1", "");
        // Should have default OpenAI compat flags
        assert!(model.compat.is_some());
    }

    // ========================================================================
    // discover_models — mock server tests
    // ========================================================================

    /// Create a reqwest client that bypasses system proxy (needed for mockito tests
    /// on machines with system-level HTTP proxy configured).
    fn no_proxy_client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    #[tokio::test]
    async fn test_discover_models_parses_openai_format() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{
                "object": "list",
                "data": [
                    {"id": "llama3.2:latest", "object": "model", "owned_by": "library", "created": 1700000000},
                    {"id": "qwen2.5-coder:7b", "object": "model", "owned_by": "library", "created": 1700000001}
                ]
            }"#)
            .create_async()
            .await;

        let client = no_proxy_client();
        let models = discover_models_with_client(&client, &server.url(), None)
            .await
            .unwrap();
        mock.assert_async().await;

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "llama3.2:latest");
        assert_eq!(models[1].id, "qwen2.5-coder:7b");
        assert_eq!(models[0].owned_by, "library");
    }

    #[tokio::test]
    async fn test_discover_models_with_api_key() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models")
            .match_header("Authorization", "Bearer test-key-123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [{"id": "gpt-4o"}]}"#)
            .create_async()
            .await;

        let client = no_proxy_client();
        let models = discover_models_with_client(&client, &server.url(), Some("test-key-123"))
            .await
            .unwrap();
        mock.assert_async().await;

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gpt-4o");
    }

    #[tokio::test]
    async fn test_discover_models_no_auth_header_when_key_is_none() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models")
            .match_header("Authorization", mockito::Matcher::Missing)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": []}"#)
            .create_async()
            .await;

        let client = no_proxy_client();
        let models = discover_models_with_client(&client, &server.url(), None)
            .await
            .unwrap();
        mock.assert_async().await;

        assert!(models.is_empty());
    }

    #[tokio::test]
    async fn test_discover_models_api_error_401() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models")
            .with_status(401)
            .with_body(r#"{"error": "unauthorized"}"#)
            .create_async()
            .await;

        let client = no_proxy_client();
        let result = discover_models_with_client(&client, &server.url(), None).await;
        mock.assert_async().await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            DiscoveryError::ApiError { status, body } => {
                assert_eq!(status, 401);
                assert!(body.contains("unauthorized"));
            }
            other => panic!("expected ApiError, got: {other}"),
        }
    }

    #[tokio::test]
    async fn test_discover_models_missing_data_field() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"models": ["a", "b"]}"#)
            .create_async()
            .await;

        let client = no_proxy_client();
        let result = discover_models_with_client(&client, &server.url(), None).await;
        mock.assert_async().await;

        assert!(result.is_err());
        match result.unwrap_err() {
            DiscoveryError::Parse(msg) => assert!(msg.contains("data")),
            other => panic!("expected Parse error, got: {other}"),
        }
    }

    #[tokio::test]
    async fn test_discover_models_connection_refused() {
        let client = no_proxy_client();
        let result = discover_models_with_client(&client, "http://127.0.0.1:1", None).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DiscoveryError::Http(_)));
    }

    #[tokio::test]
    async fn test_discover_models_trailing_slash_in_base_url() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": [{"id": "model-a"}]}"#)
            .create_async()
            .await;

        let client = no_proxy_client();
        let url = format!("{}/", server.url());
        let models = discover_models_with_client(&client, &url, None)
            .await
            .unwrap();
        mock.assert_async().await;

        assert_eq!(models.len(), 1);
    }

    #[tokio::test]
    async fn test_discover_models_skips_malformed_entries() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data": [
                {"id": "good-model"},
                {"not_id": "bad-model"},
                {"id": "another-good"}
            ]}"#,
            )
            .create_async()
            .await;

        let client = no_proxy_client();
        let models = discover_models_with_client(&client, &server.url(), None)
            .await
            .unwrap();
        mock.assert_async().await;

        // Should skip the malformed entry (missing "id" field)
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "good-model");
        assert_eq!(models[1].id, "another-good");
    }

    #[tokio::test]
    async fn test_discover_models_empty_data_array() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/models")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data": []}"#)
            .create_async()
            .await;

        let client = no_proxy_client();
        let models = discover_models_with_client(&client, &server.url(), None)
            .await
            .unwrap();
        mock.assert_async().await;

        assert!(models.is_empty());
    }
}
