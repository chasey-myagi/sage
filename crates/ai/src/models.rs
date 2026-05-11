// Built-in model catalog.
//
// Slim curated list — Anthropic + OpenAI + Google + the OpenAI-compatible
// Chinese providers we actually use (Qwen / Moonshot-Kimi / DeepSeek).
// For any model not listed here, callers should use `discover_models` against
// the provider's `/v1/models` endpoint and `construct_model_from_discovered`.

use std::sync::LazyLock;

use super::types::*;

fn anthropic(id: &str, name: &str, reasoning: bool, input_pmm: f64, output_pmm: f64) -> Model {
    Model {
        id: id.into(),
        name: name.into(),
        api: api::ANTHROPIC_MESSAGES.into(),
        provider: provider::ANTHROPIC.into(),
        base_url: "https://api.anthropic.com/v1".into(),
        api_key_env: "ANTHROPIC_API_KEY".into(),
        reasoning,
        input: vec![InputType::Text, InputType::Image],
        max_tokens: 16000,
        context_window: 200000,
        cost: ModelCost {
            input_per_million: input_pmm,
            output_per_million: output_pmm,
            cache_read_per_million: input_pmm / 10.0,
            cache_write_per_million: input_pmm * 1.25,
        },
        headers: vec![],
        compat: None,
    }
}

fn openai_compat(
    id: &str,
    name: &str,
    provider_id: &str,
    base_url: &str,
    api_key_env: &str,
    context_window: u32,
    input_pmm: f64,
    output_pmm: f64,
) -> Model {
    Model {
        id: id.into(),
        name: name.into(),
        api: api::OPENAI_COMPLETIONS.into(),
        provider: provider_id.into(),
        base_url: base_url.into(),
        api_key_env: api_key_env.into(),
        reasoning: false,
        input: vec![InputType::Text],
        max_tokens: 8192,
        context_window,
        cost: ModelCost {
            input_per_million: input_pmm,
            output_per_million: output_pmm,
            cache_read_per_million: 0.0,
            cache_write_per_million: 0.0,
        },
        headers: vec![],
        compat: Some(ProviderCompat::default()),
    }
}

fn google(id: &str, name: &str, reasoning: bool, input_pmm: f64, output_pmm: f64) -> Model {
    Model {
        id: id.into(),
        name: name.into(),
        api: api::GOOGLE_GENERATIVE_AI.into(),
        provider: provider::GOOGLE.into(),
        base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
        api_key_env: "GOOGLE_API_KEY".into(),
        reasoning,
        input: vec![InputType::Text, InputType::Image],
        max_tokens: 65536,
        context_window: 1048576,
        cost: ModelCost {
            input_per_million: input_pmm,
            output_per_million: output_pmm,
            cache_read_per_million: input_pmm / 10.0,
            cache_write_per_million: 0.0,
        },
        headers: vec![],
        compat: None,
    }
}

static MODEL_CATALOG: LazyLock<Vec<Model>> = LazyLock::new(|| {
    vec![
        // ── Anthropic ──
        anthropic("claude-opus-4-7", "Claude Opus 4.7", true, 15.0, 75.0),
        anthropic("claude-sonnet-4-6", "Claude Sonnet 4.6", true, 3.0, 15.0),
        anthropic(
            "claude-haiku-4-5-20251001",
            "Claude Haiku 4.5",
            false,
            0.8,
            4.0,
        ),
        // ── OpenAI (Chat Completions) ──
        openai_compat(
            "gpt-4o",
            "GPT-4o",
            provider::OPENAI,
            "https://api.openai.com/v1",
            "OPENAI_API_KEY",
            128000,
            2.5,
            10.0,
        ),
        openai_compat(
            "gpt-4o-mini",
            "GPT-4o Mini",
            provider::OPENAI,
            "https://api.openai.com/v1",
            "OPENAI_API_KEY",
            128000,
            0.15,
            0.6,
        ),
        // ── Google Gemini ──
        google("gemini-2.5-pro", "Gemini 2.5 Pro", true, 1.25, 10.0),
        google("gemini-2.5-flash", "Gemini 2.5 Flash", true, 0.1, 0.4),
        // ── Qwen (DashScope, OpenAI-compatible) ──
        openai_compat(
            "qwen3-235b-a22b",
            "Qwen3 235B A22B",
            provider::QWEN,
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "DASHSCOPE_API_KEY",
            131072,
            0.4,
            1.2,
        ),
        openai_compat(
            "qwen-max",
            "Qwen Max",
            provider::QWEN,
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "DASHSCOPE_API_KEY",
            32768,
            2.4,
            9.6,
        ),
        // ── Moonshot Kimi (OpenAI-compatible) ──
        openai_compat(
            "kimi-k2",
            "Kimi K2",
            provider::KIMI,
            "https://api.moonshot.cn/v1",
            "MOONSHOT_API_KEY",
            131072,
            0.6,
            2.5,
        ),
        // ── DeepSeek (OpenAI-compatible) ──
        openai_compat(
            "deepseek-chat",
            "DeepSeek V3",
            provider::DEEPSEEK,
            "https://api.deepseek.com/v1",
            "DEEPSEEK_API_KEY",
            65536,
            0.27,
            1.1,
        ),
        openai_compat(
            "deepseek-reasoner",
            "DeepSeek R1",
            provider::DEEPSEEK,
            "https://api.deepseek.com/v1",
            "DEEPSEEK_API_KEY",
            65536,
            0.55,
            2.19,
        ),
    ]
});

/// Resolves a model by provider and model ID.
pub fn resolve_model(provider: &str, model_id: &str) -> Option<Model> {
    MODEL_CATALOG
        .iter()
        .find(|m| m.provider == provider && m.id == model_id)
        .cloned()
}

/// Check if a model supports the "xhigh" thinking level (max reasoning effort).
pub fn supports_xhigh(model: &Model) -> bool {
    model.id.contains("opus")
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
        "kimi",
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
pub async fn discover_models(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<DiscoveredModel>, DiscoveryError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    discover_models_with_client(&client, base_url, api_key).await
}

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

/// Calculate the actual dollar cost for a completed request.
pub fn calculate_cost(cost_config: &ModelCost, usage: &Usage) -> Cost {
    let input = usage.input as f64 * cost_config.input_per_million / 1_000_000.0;
    let output = usage.output as f64 * cost_config.output_per_million / 1_000_000.0;
    let cache_read = usage.cache_read as f64 * cost_config.cache_read_per_million / 1_000_000.0;
    let cache_write = usage.cache_write as f64 * cost_config.cache_write_per_million / 1_000_000.0;
    Cost {
        input,
        output,
        cache_read,
        cache_write,
        total: input + output + cache_read + cache_write,
    }
}

/// Construct a usable `Model` from a `DiscoveredModel` with conservative defaults.
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
        compat: Some(ProviderCompat::default()),
    }
}
