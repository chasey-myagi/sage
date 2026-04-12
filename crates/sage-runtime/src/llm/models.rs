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
        let model = construct_model_from_discovered(&d, "ollama", "http://localhost:11434/v1", "OLLAMA_API_KEY");
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
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
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
            .with_body(r#"{"data": [
                {"id": "good-model"},
                {"not_id": "bad-model"},
                {"id": "another-good"}
            ]}"#)
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
