// Binary: generate-models
// Fetches model data from models.dev API and regenerates models.rs.
//
// Usage:
//   cargo run -p ai --bin generate-models           # print to stdout
//   cargo run -p ai --bin generate-models -- --write # overwrite crates/ai/src/models.rs

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

// ── models.dev API types ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ModelsDevModel {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    tool_call: Option<bool>,
    #[serde(default)]
    reasoning: Option<bool>,
    #[serde(default)]
    limit: Option<ModelsDevLimit>,
    #[serde(default)]
    cost: Option<ModelsDevCost>,
    #[serde(default)]
    modalities: Option<ModelsDevModalities>,
    #[serde(default)]
    provider: Option<ModelsDevProvider>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevLimit {
    #[serde(default)]
    context: Option<u64>,
    #[serde(default)]
    output: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevCost {
    #[serde(default)]
    input: Option<f64>,
    #[serde(default)]
    output: Option<f64>,
    #[serde(default)]
    cache_read: Option<f64>,
    #[serde(default)]
    cache_write: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevModalities {
    #[serde(default)]
    input: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevProvider {
    #[serde(default)]
    npm: Option<String>,
}

// ── Internal model representation ────────────────────────────────────────────

#[derive(Debug, Clone)]
struct ModelEntry {
    id: String,
    name: String,
    api: String,
    provider: String,
    base_url: String,
    api_key_env: String,
    reasoning: bool,
    input: Vec<String>,
    context_window: u64,
    max_tokens: u64,
    cost_input: f64,
    cost_output: f64,
    cost_cache_read: f64,
    cost_cache_write: f64,
    headers: Vec<(String, String)>,
}

fn api_key_for_provider(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "ANTHROPIC_API_KEY",
        "openai" | "openai-codex" => "OPENAI_API_KEY",
        "google" | "google-gemini-cli" | "google-antigravity" | "google-vertex" => "GOOGLE_API_KEY",
        "amazon-bedrock" => "AWS_ACCESS_KEY_ID",
        "groq" => "GROQ_API_KEY",
        "xai" => "XAI_API_KEY",
        "zai" => "ZAI_API_KEY",
        "mistral" => "MISTRAL_API_KEY",
        "cerebras" => "CEREBRAS_API_KEY",
        "huggingface" => "HUGGINGFACE_API_KEY",
        "github-copilot" => "GITHUB_COPILOT_TOKEN",
        "openrouter" => "OPENROUTER_API_KEY",
        "minimax" | "minimax-cn" => "MINIMAX_API_KEY",
        "kimi-coding" => "KIMI_API_KEY",
        "opencode" | "opencode-go" => "OPENCODE_API_KEY",
        "azure-openai-responses" => "AZURE_OPENAI_API_KEY",
        _ => "API_KEY",
    }
}

fn parse_provider_models(
    data: &Value,
    provider_key: &str,
    api: &str,
    provider: &str,
    base_url: &str,
    extra_skip: Option<&dyn Fn(&str, &ModelsDevModel) -> bool>,
) -> Vec<ModelEntry> {
    let mut out = Vec::new();
    let models_obj = match data.get(provider_key).and_then(|p| p.get("models")) {
        Some(v) => v,
        None => return out,
    };
    let map = match models_obj.as_object() {
        Some(m) => m,
        None => return out,
    };
    for (model_id, raw) in map {
        let m: ModelsDevModel = match serde_json::from_value(raw.clone()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if m.tool_call != Some(true) {
            continue;
        }
        if m.status.as_deref() == Some("deprecated") {
            continue;
        }
        if let Some(skip_fn) = extra_skip {
            if skip_fn(model_id, &m) {
                continue;
            }
        }
        let supports_image = m
            .modalities
            .as_ref()
            .and_then(|mo| mo.input.as_ref())
            .map(|inputs| inputs.iter().any(|i| i == "image"))
            .unwrap_or(false);
        let input = if supports_image {
            vec!["text".into(), "image".into()]
        } else {
            vec!["text".into()]
        };
        let cost = m.cost.as_ref();
        out.push(ModelEntry {
            id: model_id.clone(),
            name: m.name.clone().unwrap_or_else(|| model_id.clone()),
            api: api.into(),
            provider: provider.into(),
            base_url: base_url.into(),
            api_key_env: api_key_for_provider(provider).into(),
            reasoning: m.reasoning == Some(true),
            input,
            context_window: m.limit.as_ref().and_then(|l| l.context).unwrap_or(4096),
            max_tokens: m.limit.as_ref().and_then(|l| l.output).unwrap_or(4096),
            cost_input: cost.and_then(|c| c.input).unwrap_or(0.0),
            cost_output: cost.and_then(|c| c.output).unwrap_or(0.0),
            cost_cache_read: cost.and_then(|c| c.cache_read).unwrap_or(0.0),
            cost_cache_write: cost.and_then(|c| c.cache_write).unwrap_or(0.0),
            headers: vec![],
        });
    }
    out
}

async fn fetch_models_dev() -> Result<Vec<ModelEntry>> {
    eprintln!("Fetching models from models.dev API...");
    let client = reqwest::Client::new();
    let data: Value = client
        .get("https://models.dev/api.json")
        .send()
        .await
        .context("GET models.dev/api.json")?
        .json()
        .await
        .context("parse models.dev JSON")?;

    let mut models: Vec<ModelEntry> = Vec::new();

    // Amazon Bedrock
    models.extend(parse_provider_models(
        &data,
        "amazon-bedrock",
        "bedrock-converse-stream",
        "amazon-bedrock",
        "https://bedrock-runtime.us-east-1.amazonaws.com",
        Some(&|id, _m| {
            id.starts_with("ai21.jamba") || id.starts_with("mistral.mistral-7b-instruct-v0")
        }),
    ));

    // Anthropic
    models.extend(parse_provider_models(
        &data,
        "anthropic",
        "anthropic-messages",
        "anthropic",
        "https://api.anthropic.com/v1",
        None,
    ));

    // Google
    models.extend(parse_provider_models(
        &data,
        "google",
        "google-generative-ai",
        "google",
        "https://generativelanguage.googleapis.com/v1beta",
        None,
    ));

    // OpenAI
    models.extend(parse_provider_models(
        &data,
        "openai",
        "openai-responses",
        "openai",
        "https://api.openai.com/v1",
        None,
    ));

    // Groq
    models.extend(parse_provider_models(
        &data,
        "groq",
        "openai-completions",
        "groq",
        "https://api.groq.com/openai/v1",
        None,
    ));

    // Cerebras
    models.extend(parse_provider_models(
        &data,
        "cerebras",
        "openai-completions",
        "cerebras",
        "https://api.cerebras.ai/v1",
        None,
    ));

    // xAI
    models.extend(parse_provider_models(
        &data,
        "xai",
        "openai-completions",
        "xai",
        "https://api.x.ai/v1",
        None,
    ));

    // zAI
    {
        let zai_models = parse_provider_models(
            &data,
            "zai",
            "openai-completions",
            "zai",
            "https://api.z.ai/api/coding/paas/v4",
            None,
        );
        models.extend(zai_models);
    }

    // Mistral
    models.extend(parse_provider_models(
        &data,
        "mistral",
        "mistral-conversations",
        "mistral",
        "https://api.mistral.ai",
        None,
    ));

    // HuggingFace
    models.extend(parse_provider_models(
        &data,
        "huggingface",
        "openai-completions",
        "huggingface",
        "https://router.huggingface.co/v1",
        None,
    ));

    // OpenCode variants
    for (key, provider, base_path) in &[
        ("opencode", "opencode", "https://opencode.ai/zen"),
        ("opencode-go", "opencode-go", "https://opencode.ai/zen/go"),
    ] {
        let models_obj = match data.get(*key).and_then(|p| p.get("models")) {
            Some(v) => v,
            None => continue,
        };
        let map = match models_obj.as_object() {
            Some(m) => m,
            None => continue,
        };
        for (model_id, raw) in map {
            let m: ModelsDevModel = match serde_json::from_value(raw.clone()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if m.tool_call != Some(true) {
                continue;
            }
            if m.status.as_deref() == Some("deprecated") {
                continue;
            }
            let npm = m.provider.as_ref().and_then(|p| p.npm.as_deref());
            let (api, base_url) = match npm {
                Some("@ai-sdk/openai") => ("openai-responses", format!("{}/v1", base_path)),
                Some("@ai-sdk/anthropic") => ("anthropic-messages", base_path.to_string()),
                Some("@ai-sdk/google") => ("google-generative-ai", format!("{}/v1", base_path)),
                _ => ("openai-completions", format!("{}/v1", base_path)),
            };
            let supports_image = m
                .modalities
                .as_ref()
                .and_then(|mo| mo.input.as_ref())
                .map(|inputs| inputs.iter().any(|i| i == "image"))
                .unwrap_or(false);
            let input = if supports_image {
                vec!["text".into(), "image".into()]
            } else {
                vec!["text".into()]
            };
            let cost = m.cost.as_ref();
            models.push(ModelEntry {
                id: model_id.clone(),
                name: m.name.clone().unwrap_or_else(|| model_id.clone()),
                api: api.into(),
                provider: provider.to_string(),
                base_url,
                api_key_env: api_key_for_provider(provider).into(),
                reasoning: m.reasoning == Some(true),
                input,
                context_window: m.limit.as_ref().and_then(|l| l.context).unwrap_or(4096),
                max_tokens: m.limit.as_ref().and_then(|l| l.output).unwrap_or(4096),
                cost_input: cost.and_then(|c| c.input).unwrap_or(0.0),
                cost_output: cost.and_then(|c| c.output).unwrap_or(0.0),
                cost_cache_read: cost.and_then(|c| c.cache_read).unwrap_or(0.0),
                cost_cache_write: cost.and_then(|c| c.cache_write).unwrap_or(0.0),
                headers: vec![],
            });
        }
    }

    // GitHub Copilot
    {
        let copilot_headers: Vec<(String, String)> = vec![
            ("User-Agent".into(), "GitHubCopilotChat/0.35.0".into()),
            ("Editor-Version".into(), "vscode/1.107.0".into()),
            ("Editor-Plugin-Version".into(), "copilot-chat/0.35.0".into()),
            ("Copilot-Integration-Id".into(), "vscode-chat".into()),
        ];
        let models_obj = match data.get("github-copilot").and_then(|p| p.get("models")) {
            Some(v) => v,
            None => &Value::Null,
        };
        if let Some(map) = models_obj.as_object() {
            for (model_id, raw) in map {
                let m: ModelsDevModel = match serde_json::from_value(raw.clone()) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if m.tool_call != Some(true) {
                    continue;
                }
                if m.status.as_deref() == Some("deprecated") {
                    continue;
                }
                // Claude 4.x → anthropic-messages, gpt-5/oswe → openai-responses, else openai-completions
                let is_claude4 = regex_matches_claude4(model_id);
                let needs_responses = model_id.starts_with("gpt-5") || model_id.starts_with("oswe");
                let api = if is_claude4 {
                    "anthropic-messages"
                } else if needs_responses {
                    "openai-responses"
                } else {
                    "openai-completions"
                };
                let supports_image = m
                    .modalities
                    .as_ref()
                    .and_then(|mo| mo.input.as_ref())
                    .map(|inputs| inputs.iter().any(|i| i == "image"))
                    .unwrap_or(false);
                let input = if supports_image {
                    vec!["text".into(), "image".into()]
                } else {
                    vec!["text".into()]
                };
                let cost = m.cost.as_ref();
                models.push(ModelEntry {
                    id: model_id.clone(),
                    name: m.name.clone().unwrap_or_else(|| model_id.clone()),
                    api: api.into(),
                    provider: "github-copilot".into(),
                    base_url: "https://api.individual.githubcopilot.com".into(),
                    api_key_env: "GITHUB_COPILOT_TOKEN".into(),
                    reasoning: m.reasoning == Some(true),
                    input,
                    context_window: m.limit.as_ref().and_then(|l| l.context).unwrap_or(128000),
                    max_tokens: m.limit.as_ref().and_then(|l| l.output).unwrap_or(8192),
                    cost_input: cost.and_then(|c| c.input).unwrap_or(0.0),
                    cost_output: cost.and_then(|c| c.output).unwrap_or(0.0),
                    cost_cache_read: cost.and_then(|c| c.cache_read).unwrap_or(0.0),
                    cost_cache_write: cost.and_then(|c| c.cache_write).unwrap_or(0.0),
                    headers: copilot_headers.clone(),
                });
            }
        }
    }

    // MiniMax variants
    for (key, provider, base_url) in &[
        ("minimax", "minimax", "https://api.minimax.io/anthropic"),
        (
            "minimax-cn",
            "minimax-cn",
            "https://api.minimaxi.com/anthropic",
        ),
    ] {
        models.extend(parse_provider_models(
            &data,
            key,
            "anthropic-messages",
            provider,
            base_url,
            None,
        ));
    }

    // Kimi For Coding
    models.extend(parse_provider_models(
        &data,
        "kimi-for-coding",
        "anthropic-messages",
        "kimi-coding",
        "https://api.kimi.com/coding",
        None,
    ));

    eprintln!(
        "Loaded {} tool-capable models from models.dev",
        models.len()
    );
    Ok(models)
}

fn regex_matches_claude4(id: &str) -> bool {
    // claude-(haiku|sonnet|opus)-4[.\-] or claude-...-4 at end
    let pat = ["claude-haiku-4", "claude-sonnet-4", "claude-opus-4"];
    for p in &pat {
        if id.starts_with(p) {
            let rest = &id[p.len()..];
            if rest.is_empty() || rest.starts_with('.') || rest.starts_with('-') {
                return true;
            }
        }
    }
    false
}

fn has_model(models: &[ModelEntry], provider: &str, id: &str) -> bool {
    models.iter().any(|m| m.provider == provider && m.id == id)
}

fn add_hardcoded_models(models: &mut Vec<ModelEntry>) {
    // ── Apply upstream fixes ────────────────────────────────────────────────

    // Fix incorrect cache pricing for Claude Opus 4.5
    for m in models.iter_mut() {
        if m.provider == "anthropic" && m.id == "claude-opus-4-5" {
            m.cost_cache_read = 0.5;
            m.cost_cache_write = 6.25;
        }
        if m.provider == "amazon-bedrock" && m.id.contains("anthropic.claude-opus-4-6-v1") {
            m.cost_cache_read = 0.5;
            m.cost_cache_write = 6.25;
        }
        // Context window overrides for 4.6 models
        if matches!(
            m.provider.as_str(),
            "anthropic" | "opencode" | "opencode-go" | "github-copilot"
        ) && matches!(
            m.id.as_str(),
            "claude-opus-4-6" | "claude-sonnet-4-6" | "claude-opus-4.6" | "claude-sonnet-4.6"
        ) {
            m.context_window = 1000000;
        }
        if m.provider == "google-antigravity"
            && matches!(
                m.id.as_str(),
                "claude-opus-4-6-thinking" | "claude-sonnet-4-6"
            )
        {
            m.context_window = 1000000;
        }
        // OpenCode variants: Claude Sonnet 4/4.5 actual limit is 200K
        if matches!(m.provider.as_str(), "opencode" | "opencode-go")
            && matches!(m.id.as_str(), "claude-sonnet-4-5" | "claude-sonnet-4")
        {
            m.context_window = 200000;
        }
        if matches!(m.provider.as_str(), "opencode" | "opencode-go") && m.id == "gpt-5.4" {
            m.context_window = 272000;
            m.max_tokens = 128000;
        }
        if m.provider == "openai" && m.id == "gpt-5.4" {
            m.context_window = 272000;
            m.max_tokens = 128000;
        }
    }

    // MiniMax: only keep supported IDs, fix context/max_tokens
    let minimax_supported: std::collections::HashSet<&str> =
        ["MiniMax-M2.7", "MiniMax-M2.7-highspeed"].into();
    models.retain(|m| {
        if matches!(m.provider.as_str(), "minimax" | "minimax-cn") {
            minimax_supported.contains(m.id.as_str())
        } else {
            true
        }
    });
    for m in models.iter_mut() {
        if matches!(m.provider.as_str(), "minimax" | "minimax-cn")
            && minimax_supported.contains(m.id.as_str())
        {
            m.context_window = 204800;
            m.max_tokens = 131072;
        }
    }

    // Filter out opencode gpt-5.3-codex-spark
    models.retain(|m| {
        !(matches!(m.provider.as_str(), "opencode" | "opencode-go")
            && m.id == "gpt-5.3-codex-spark")
    });

    // ── Missing Bedrock EU profile ──────────────────────────────────────────
    if !has_model(models, "amazon-bedrock", "eu.anthropic.claude-opus-4-6-v1") {
        models.push(ModelEntry {
            id: "eu.anthropic.claude-opus-4-6-v1".into(),
            name: "Claude Opus 4.6 (EU)".into(),
            api: "bedrock-converse-stream".into(),
            provider: "amazon-bedrock".into(),
            base_url: "https://bedrock-runtime.us-east-1.amazonaws.com".into(),
            api_key_env: "AWS_ACCESS_KEY_ID".into(),
            reasoning: true,
            input: vec!["text".into(), "image".into()],
            context_window: 200000,
            max_tokens: 128000,
            cost_input: 5.0,
            cost_output: 25.0,
            cost_cache_read: 0.5,
            cost_cache_write: 6.25,
            headers: vec![],
        });
    }

    // ── Missing Anthropic models ────────────────────────────────────────────
    if !has_model(models, "anthropic", "claude-opus-4-6") {
        models.push(ModelEntry {
            id: "claude-opus-4-6".into(),
            name: "Claude Opus 4.6".into(),
            api: "anthropic-messages".into(),
            provider: "anthropic".into(),
            base_url: "https://api.anthropic.com/v1".into(),
            api_key_env: "ANTHROPIC_API_KEY".into(),
            reasoning: true,
            input: vec!["text".into(), "image".into()],
            context_window: 1000000,
            max_tokens: 128000,
            cost_input: 5.0,
            cost_output: 25.0,
            cost_cache_read: 0.5,
            cost_cache_write: 6.25,
            headers: vec![],
        });
    }
    if !has_model(models, "anthropic", "claude-sonnet-4-6") {
        models.push(ModelEntry {
            id: "claude-sonnet-4-6".into(),
            name: "Claude Sonnet 4.6".into(),
            api: "anthropic-messages".into(),
            provider: "anthropic".into(),
            base_url: "https://api.anthropic.com/v1".into(),
            api_key_env: "ANTHROPIC_API_KEY".into(),
            reasoning: true,
            input: vec!["text".into(), "image".into()],
            context_window: 1000000,
            max_tokens: 64000,
            cost_input: 3.0,
            cost_output: 15.0,
            cost_cache_read: 0.3,
            cost_cache_write: 3.75,
            headers: vec![],
        });
    }

    // ── Missing Google models ───────────────────────────────────────────────
    if !has_model(models, "google", "gemini-3.1-flash-lite-preview") {
        models.push(ModelEntry {
            id: "gemini-3.1-flash-lite-preview".into(),
            name: "Gemini 3.1 Flash Lite Preview".into(),
            api: "google-generative-ai".into(),
            provider: "google".into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            api_key_env: "GOOGLE_API_KEY".into(),
            reasoning: true,
            input: vec!["text".into(), "image".into()],
            context_window: 1048576,
            max_tokens: 65536,
            cost_input: 0.0,
            cost_output: 0.0,
            cost_cache_read: 0.0,
            cost_cache_write: 0.0,
            headers: vec![],
        });
    }

    // ── Missing OpenAI models ───────────────────────────────────────────────
    let openai_extra: &[(&str, &str, bool, &[&str], u64, u64, f64, f64, f64, f64)] = &[
        (
            "gpt-5-chat-latest",
            "GPT-5 Chat Latest",
            false,
            &["text", "image"],
            128000,
            16384,
            1.25,
            10.0,
            0.125,
            0.0,
        ),
        (
            "gpt-5.1-codex",
            "GPT-5.1 Codex",
            true,
            &["text", "image"],
            400000,
            128000,
            1.25,
            5.0,
            0.125,
            1.25,
        ),
        (
            "gpt-5.1-codex-max",
            "GPT-5.1 Codex Max",
            true,
            &["text", "image"],
            400000,
            128000,
            1.25,
            10.0,
            0.125,
            0.0,
        ),
        (
            "gpt-5.3-codex-spark",
            "GPT-5.3 Codex Spark",
            true,
            &["text"],
            128000,
            16384,
            0.0,
            0.0,
            0.0,
            0.0,
        ),
        (
            "gpt-5.4",
            "GPT-5.4",
            true,
            &["text", "image"],
            272000,
            128000,
            2.5,
            15.0,
            0.25,
            0.0,
        ),
    ];
    for &(id, name, reasoning, input, ctx, max_tok, ci, co, cr, cw) in openai_extra {
        if !has_model(models, "openai", id) {
            models.push(ModelEntry {
                id: id.into(),
                name: name.into(),
                api: "openai-responses".into(),
                provider: "openai".into(),
                base_url: "https://api.openai.com/v1".into(),
                api_key_env: "OPENAI_API_KEY".into(),
                reasoning,
                input: input.iter().map(|s| s.to_string()).collect(),
                context_window: ctx,
                max_tokens: max_tok,
                cost_input: ci,
                cost_output: co,
                cost_cache_read: cr,
                cost_cache_write: cw,
                headers: vec![],
            });
        }
    }

    // ── GitHub Copilot: add gpt-5.3-codex if gpt-5.2-codex exists ──────────
    let has_copilot_523 = has_model(models, "github-copilot", "gpt-5.2-codex");
    let has_copilot_533 = has_model(models, "github-copilot", "gpt-5.3-codex");
    if has_copilot_523 && !has_copilot_533 {
        if let Some(base) = models
            .iter()
            .find(|m| m.provider == "github-copilot" && m.id == "gpt-5.2-codex")
            .cloned()
        {
            models.push(ModelEntry {
                id: "gpt-5.3-codex".into(),
                name: "GPT-5.3 Codex".into(),
                ..base
            });
        }
    }

    // ── xAI: missing Grok Code Fast ────────────────────────────────────────
    if !has_model(models, "xai", "grok-code-fast-1") {
        models.push(ModelEntry {
            id: "grok-code-fast-1".into(),
            name: "Grok Code Fast 1".into(),
            api: "openai-completions".into(),
            provider: "xai".into(),
            base_url: "https://api.x.ai/v1".into(),
            api_key_env: "XAI_API_KEY".into(),
            reasoning: false,
            input: vec!["text".into()],
            context_window: 32768,
            max_tokens: 8192,
            cost_input: 0.2,
            cost_output: 1.5,
            cost_cache_read: 0.02,
            cost_cache_write: 0.0,
            headers: vec![],
        });
    }

    // ── OpenRouter: add auto alias ──────────────────────────────────────────
    if !has_model(models, "openrouter", "auto") {
        models.push(ModelEntry {
            id: "auto".into(),
            name: "Auto".into(),
            api: "openai-completions".into(),
            provider: "openrouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            api_key_env: "OPENROUTER_API_KEY".into(),
            reasoning: true,
            input: vec!["text".into(), "image".into()],
            context_window: 2000000,
            max_tokens: 30000,
            cost_input: 0.0,
            cost_output: 0.0,
            cost_cache_read: 0.0,
            cost_cache_write: 0.0,
            headers: vec![],
        });
    }

    // ── OpenAI Codex (ChatGPT OAuth) models ────────────────────────────────
    let codex_base_url = "https://chatgpt.com/backend-api";
    let codex_ctx = 272000u64;
    let codex_max = 128000u64;
    let codex_models: &[(&str, &str, f64, f64, f64, f64, &[&str], u64)] = &[
        (
            "gpt-5.1",
            "GPT-5.1",
            1.25,
            10.0,
            0.125,
            0.0,
            &["text", "image"],
            codex_ctx,
        ),
        (
            "gpt-5.1-codex-max",
            "GPT-5.1 Codex Max",
            1.25,
            10.0,
            0.125,
            0.0,
            &["text", "image"],
            codex_ctx,
        ),
        (
            "gpt-5.1-codex-mini",
            "GPT-5.1 Codex Mini",
            0.25,
            2.0,
            0.025,
            0.0,
            &["text", "image"],
            codex_ctx,
        ),
        (
            "gpt-5.2",
            "GPT-5.2",
            1.75,
            14.0,
            0.175,
            0.0,
            &["text", "image"],
            codex_ctx,
        ),
        (
            "gpt-5.2-codex",
            "GPT-5.2 Codex",
            1.75,
            14.0,
            0.175,
            0.0,
            &["text", "image"],
            codex_ctx,
        ),
        (
            "gpt-5.3-codex",
            "GPT-5.3 Codex",
            1.75,
            14.0,
            0.175,
            0.0,
            &["text", "image"],
            codex_ctx,
        ),
        (
            "gpt-5.4",
            "GPT-5.4",
            2.5,
            15.0,
            0.25,
            0.0,
            &["text", "image"],
            codex_ctx,
        ),
        (
            "gpt-5.4-mini",
            "GPT-5.4 Mini",
            0.75,
            4.5,
            0.075,
            0.0,
            &["text", "image"],
            codex_ctx,
        ),
        (
            "gpt-5.3-codex-spark",
            "GPT-5.3 Codex Spark",
            0.0,
            0.0,
            0.0,
            0.0,
            &["text"],
            128000,
        ),
    ];
    for &(id, name, ci, co, cr, cw, input, ctx) in codex_models {
        if !has_model(models, "openai-codex", id) {
            models.push(ModelEntry {
                id: id.into(),
                name: name.into(),
                api: "openai-codex-responses".into(),
                provider: "openai-codex".into(),
                base_url: codex_base_url.into(),
                api_key_env: "OPENAI_API_KEY".into(),
                reasoning: true,
                input: input.iter().map(|s| s.to_string()).collect(),
                context_window: ctx,
                max_tokens: codex_max,
                cost_input: ci,
                cost_output: co,
                cost_cache_read: cr,
                cost_cache_write: cw,
                headers: vec![],
            });
        }
    }

    // ── Google Cloud Code Assist ────────────────────────────────────────────
    let cloud_code_endpoint = "https://cloudcode-pa.googleapis.com";
    let cloud_code_models: &[(&str, &str, bool, u64)] = &[
        (
            "gemini-2.5-pro",
            "Gemini 2.5 Pro (Cloud Code Assist)",
            true,
            65535,
        ),
        (
            "gemini-2.5-flash",
            "Gemini 2.5 Flash (Cloud Code Assist)",
            true,
            65535,
        ),
        (
            "gemini-2.0-flash",
            "Gemini 2.0 Flash (Cloud Code Assist)",
            false,
            8192,
        ),
        (
            "gemini-3-pro-preview",
            "Gemini 3 Pro Preview (Cloud Code Assist)",
            true,
            65535,
        ),
        (
            "gemini-3-flash-preview",
            "Gemini 3 Flash Preview (Cloud Code Assist)",
            true,
            65535,
        ),
        (
            "gemini-3.1-pro-preview",
            "Gemini 3.1 Pro Preview (Cloud Code Assist)",
            true,
            65535,
        ),
    ];
    for &(id, name, reasoning, max_tok) in cloud_code_models {
        if !has_model(models, "google-gemini-cli", id) {
            models.push(ModelEntry {
                id: id.into(),
                name: name.into(),
                api: "google-gemini-cli".into(),
                provider: "google-gemini-cli".into(),
                base_url: cloud_code_endpoint.into(),
                api_key_env: "GOOGLE_API_KEY".into(),
                reasoning,
                input: vec!["text".into(), "image".into()],
                context_window: 1048576,
                max_tokens: max_tok,
                cost_input: 0.0,
                cost_output: 0.0,
                cost_cache_read: 0.0,
                cost_cache_write: 0.0,
                headers: vec![],
            });
        }
    }

    // ── Antigravity models ──────────────────────────────────────────────────
    let antigravity_endpoint = "https://daily-cloudcode-pa.sandbox.googleapis.com";
    let antigravity_models: &[(&str, &str, bool, &[&str], u64, u64, f64, f64, f64, f64)] = &[
        (
            "gemini-3.1-pro-high",
            "Gemini 3.1 Pro High (Antigravity)",
            true,
            &["text", "image"],
            1048576,
            65535,
            2.0,
            12.0,
            0.2,
            2.375,
        ),
        (
            "gemini-3.1-pro-low",
            "Gemini 3.1 Pro Low (Antigravity)",
            true,
            &["text", "image"],
            1048576,
            65535,
            2.0,
            12.0,
            0.2,
            2.375,
        ),
        (
            "gemini-3-flash",
            "Gemini 3 Flash (Antigravity)",
            true,
            &["text", "image"],
            1048576,
            65535,
            0.5,
            3.0,
            0.5,
            0.0,
        ),
        (
            "claude-sonnet-4-5",
            "Claude Sonnet 4.5 (Antigravity)",
            false,
            &["text", "image"],
            200000,
            64000,
            3.0,
            15.0,
            0.3,
            3.75,
        ),
        (
            "claude-sonnet-4-5-thinking",
            "Claude Sonnet 4.5 Thinking (Antigravity)",
            true,
            &["text", "image"],
            200000,
            64000,
            3.0,
            15.0,
            0.3,
            3.75,
        ),
        (
            "claude-opus-4-5-thinking",
            "Claude Opus 4.5 Thinking (Antigravity)",
            true,
            &["text", "image"],
            200000,
            64000,
            5.0,
            25.0,
            0.5,
            6.25,
        ),
        (
            "claude-opus-4-6-thinking",
            "Claude Opus 4.6 Thinking (Antigravity)",
            true,
            &["text", "image"],
            200000,
            128000,
            5.0,
            25.0,
            0.5,
            6.25,
        ),
        (
            "claude-sonnet-4-6",
            "Claude Sonnet 4.6 (Antigravity)",
            true,
            &["text", "image"],
            200000,
            64000,
            3.0,
            15.0,
            0.3,
            3.75,
        ),
        (
            "gpt-oss-120b-medium",
            "GPT-OSS 120B Medium (Antigravity)",
            false,
            &["text"],
            131072,
            32768,
            0.09,
            0.36,
            0.0,
            0.0,
        ),
    ];
    for &(id, name, reasoning, input, ctx, max_tok, ci, co, cr, cw) in antigravity_models {
        models.push(ModelEntry {
            id: id.into(),
            name: name.into(),
            api: "google-gemini-cli".into(),
            provider: "google-antigravity".into(),
            base_url: antigravity_endpoint.into(),
            api_key_env: "GOOGLE_API_KEY".into(),
            reasoning,
            input: input.iter().map(|s| s.to_string()).collect(),
            context_window: ctx,
            max_tokens: max_tok,
            cost_input: ci,
            cost_output: co,
            cost_cache_read: cr,
            cost_cache_write: cw,
            headers: vec![],
        });
    }
    // Apply context override for antigravity 4.6 models
    for m in models.iter_mut() {
        if m.provider == "google-antigravity"
            && matches!(
                m.id.as_str(),
                "claude-opus-4-6-thinking" | "claude-sonnet-4-6"
            )
        {
            m.context_window = 1000000;
        }
    }

    // ── Vertex AI models ────────────────────────────────────────────────────
    let vertex_base_url = "https://{location}-aiplatform.googleapis.com";
    let vertex_models: &[(&str, &str, bool, u64, u64, f64, f64, f64, f64)] = &[
        (
            "gemini-3-pro-preview",
            "Gemini 3 Pro Preview (Vertex)",
            true,
            1000000,
            64000,
            2.0,
            12.0,
            0.2,
            0.0,
        ),
        (
            "gemini-3.1-pro-preview",
            "Gemini 3.1 Pro Preview (Vertex)",
            true,
            1048576,
            65536,
            2.0,
            12.0,
            0.2,
            0.0,
        ),
        (
            "gemini-3-flash-preview",
            "Gemini 3 Flash Preview (Vertex)",
            true,
            1048576,
            65536,
            0.5,
            3.0,
            0.05,
            0.0,
        ),
        (
            "gemini-2.0-flash",
            "Gemini 2.0 Flash (Vertex)",
            false,
            1048576,
            8192,
            0.15,
            0.6,
            0.0375,
            0.0,
        ),
        (
            "gemini-2.0-flash-lite",
            "Gemini 2.0 Flash Lite (Vertex)",
            true,
            1048576,
            65536,
            0.075,
            0.3,
            0.01875,
            0.0,
        ),
        (
            "gemini-2.5-pro",
            "Gemini 2.5 Pro (Vertex)",
            true,
            1048576,
            65536,
            1.25,
            10.0,
            0.125,
            0.0,
        ),
        (
            "gemini-2.5-flash",
            "Gemini 2.5 Flash (Vertex)",
            true,
            1048576,
            65536,
            0.3,
            2.5,
            0.03,
            0.0,
        ),
        (
            "gemini-2.5-flash-lite-preview-09-2025",
            "Gemini 2.5 Flash Lite Preview 09-25 (Vertex)",
            true,
            1048576,
            65536,
            0.1,
            0.4,
            0.01,
            0.0,
        ),
        (
            "gemini-2.5-flash-lite",
            "Gemini 2.5 Flash Lite (Vertex)",
            true,
            1048576,
            65536,
            0.1,
            0.4,
            0.01,
            0.0,
        ),
        (
            "gemini-1.5-pro",
            "Gemini 1.5 Pro (Vertex)",
            false,
            1000000,
            8192,
            1.25,
            5.0,
            0.3125,
            0.0,
        ),
        (
            "gemini-1.5-flash",
            "Gemini 1.5 Flash (Vertex)",
            false,
            1000000,
            8192,
            0.075,
            0.3,
            0.01875,
            0.0,
        ),
        (
            "gemini-1.5-flash-8b",
            "Gemini 1.5 Flash-8B (Vertex)",
            false,
            1000000,
            8192,
            0.0375,
            0.15,
            0.01,
            0.0,
        ),
    ];
    for &(id, name, reasoning, ctx, max_tok, ci, co, cr, cw) in vertex_models {
        models.push(ModelEntry {
            id: id.into(),
            name: name.into(),
            api: "google-vertex".into(),
            provider: "google-vertex".into(),
            base_url: vertex_base_url.into(),
            api_key_env: "GOOGLE_API_KEY".into(),
            reasoning,
            input: vec!["text".into(), "image".into()],
            context_window: ctx,
            max_tokens: max_tok,
            cost_input: ci,
            cost_output: co,
            cost_cache_read: cr,
            cost_cache_write: cw,
            headers: vec![],
        });
    }

    // ── Kimi For Coding static fallback ────────────────────────────────────
    let kimi_models: &[(&str, &str)] = &[
        ("kimi-k2-thinking", "Kimi K2 Thinking"),
        ("k2p5", "Kimi K2.5"),
    ];
    for &(id, name) in kimi_models {
        if !has_model(models, "kimi-coding", id) {
            models.push(ModelEntry {
                id: id.into(),
                name: name.into(),
                api: "anthropic-messages".into(),
                provider: "kimi-coding".into(),
                base_url: "https://api.kimi.com/coding".into(),
                api_key_env: "KIMI_API_KEY".into(),
                reasoning: true,
                input: vec!["text".into()],
                context_window: 262144,
                max_tokens: 32768,
                cost_input: 0.0,
                cost_output: 0.0,
                cost_cache_read: 0.0,
                cost_cache_write: 0.0,
                headers: vec![],
            });
        }
    }

    // ── Azure OpenAI: mirror OpenAI responses models ────────────────────────
    let openai_responses: Vec<ModelEntry> = models
        .iter()
        .filter(|m| m.provider == "openai" && m.api == "openai-responses")
        .cloned()
        .collect();
    for mut m in openai_responses {
        if !has_model(models, "azure-openai-responses", &m.id) {
            m.api = "azure-openai-responses".into();
            m.provider = "azure-openai-responses".into();
            m.base_url = String::new();
            m.api_key_env = "AZURE_OPENAI_API_KEY".into();
            models.push(m);
        }
    }
}

// ── Code generation ───────────────────────────────────────────────────────────

fn format_f64(v: f64) -> String {
    // Produce clean output: no trailing zeros beyond what's needed, but always has decimal
    if v == 0.0 {
        return "0.0".into();
    }
    // Use up to 6 sig digits, strip trailing zeros but keep at least one decimal place
    let s = format!("{:.6}", v);
    let s = s.trim_end_matches('0');
    if s.ends_with('.') {
        format!("{}0", s)
    } else {
        s.to_string()
    }
}

fn generate_model_code(m: &ModelEntry, indent: &str) -> String {
    let mut s = String::new();
    let i = indent;
    let i1 = format!("{}    ", i);
    let i2 = format!("{}        ", i);

    writeln!(s, "{}Model {{", i).unwrap();
    writeln!(s, "{}id: {:?}.into(),", i1, m.id).unwrap();
    writeln!(s, "{}name: {:?}.into(),", i1, m.name).unwrap();
    writeln!(s, "{}api: {:?}.into(),", i1, m.api).unwrap();
    writeln!(s, "{}provider: {:?}.into(),", i1, m.provider).unwrap();
    writeln!(s, "{}base_url: {:?}.into(),", i1, m.base_url).unwrap();
    writeln!(s, "{}api_key_env: {:?}.into(),", i1, m.api_key_env).unwrap();
    writeln!(s, "{}reasoning: {},", i1, m.reasoning).unwrap();

    // input vec
    let input_items: Vec<String> = m
        .input
        .iter()
        .map(|t| {
            if t == "image" {
                "InputType::Image".into()
            } else {
                "InputType::Text".into()
            }
        })
        .collect();
    writeln!(s, "{}input: vec![{}],", i1, input_items.join(", ")).unwrap();

    writeln!(s, "{}max_tokens: {},", i1, m.max_tokens).unwrap();
    writeln!(s, "{}context_window: {},", i1, m.context_window).unwrap();
    writeln!(s, "{}cost: ModelCost {{", i1).unwrap();
    writeln!(s, "{}input_per_million: {},", i2, format_f64(m.cost_input)).unwrap();
    writeln!(
        s,
        "{}output_per_million: {},",
        i2,
        format_f64(m.cost_output)
    )
    .unwrap();
    writeln!(
        s,
        "{}cache_read_per_million: {},",
        i2,
        format_f64(m.cost_cache_read)
    )
    .unwrap();
    writeln!(
        s,
        "{}cache_write_per_million: {},",
        i2,
        format_f64(m.cost_cache_write)
    )
    .unwrap();
    writeln!(s, "{}}},", i1).unwrap();

    if m.headers.is_empty() {
        writeln!(s, "{}headers: vec![],", i1).unwrap();
    } else {
        writeln!(s, "{}headers: vec![", i1).unwrap();
        for (k, v) in &m.headers {
            writeln!(s, "{}({:?}.into(), {:?}.into()),", i2, k, v).unwrap();
        }
        writeln!(s, "{}],", i1).unwrap();
    }

    writeln!(s, "{}compat: None,", i1).unwrap();
    write!(s, "{}}}", i).unwrap();
    s
}

fn generate_output(models: &[ModelEntry]) -> String {
    // Deduplicate by (provider, id) — first wins
    let mut seen: HashMap<(String, String), bool> = HashMap::new();
    let deduped: Vec<&ModelEntry> = models
        .iter()
        .filter(|m| {
            let key = (m.provider.clone(), m.id.clone());
            if seen.contains_key(&key) {
                false
            } else {
                seen.insert(key, true);
                true
            }
        })
        .collect();

    // Group by provider, sort for determinism
    let mut by_provider: HashMap<&str, Vec<&ModelEntry>> = HashMap::new();
    for m in &deduped {
        by_provider.entry(m.provider.as_str()).or_default().push(m);
    }
    let mut provider_ids: Vec<&str> = by_provider.keys().copied().collect();
    provider_ids.sort();

    let mut out = String::new();
    writeln!(
        out,
        "// This file is auto-generated by `cargo run -p ai --bin generate-models`"
    )
    .unwrap();
    writeln!(out, "// Do not edit manually.").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "use std::sync::LazyLock;").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "use super::types::*;").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "static MODEL_CATALOG: LazyLock<Vec<Model>> = LazyLock::new(|| {{"
    )
    .unwrap();
    writeln!(out, "    vec![").unwrap();

    for provider_id in &provider_ids {
        let mut provider_models = by_provider[provider_id].clone();
        provider_models.sort_by(|a, b| a.id.cmp(&b.id));

        writeln!(out, "        // ── {} ──", provider_id).unwrap();
        for m in provider_models {
            let code = generate_model_code(m, "        ");
            writeln!(out, "{},", code).unwrap();
        }
    }

    writeln!(out, "    ]").unwrap();
    writeln!(out, "}});").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "pub fn all_models() -> &'static [Model] {{").unwrap();
    writeln!(out, "    &MODEL_CATALOG").unwrap();
    writeln!(out, "}}").unwrap();

    out
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let write_flag = std::env::args().any(|a| a == "--write");

    let mut models = fetch_models_dev().await?;
    add_hardcoded_models(&mut models);

    let total = models.len();
    let reasoning = models.iter().filter(|m| m.reasoning).count();
    eprintln!("\nModel statistics:");
    eprintln!("  Total tool-capable models: {}", total);
    eprintln!("  Reasoning-capable models: {}", reasoning);

    // Print per-provider counts
    let mut by_provider: HashMap<&str, usize> = HashMap::new();
    for m in &models {
        *by_provider.entry(m.provider.as_str()).or_default() += 1;
    }
    let mut providers: Vec<(&str, usize)> = by_provider.into_iter().collect();
    providers.sort_by_key(|(p, _)| *p);
    for (p, count) in providers {
        eprintln!("  {}: {} models", p, count);
    }

    let output = generate_output(&models);

    if write_flag {
        // Write to crates/ai/src/models.rs relative to this binary's manifest dir.
        // At build time the env var CARGO_MANIFEST_DIR points to crates/ai.
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let out_path = std::path::Path::new(manifest_dir).join("src/models.rs");
        std::fs::write(&out_path, &output)
            .with_context(|| format!("write {}", out_path.display()))?;
        eprintln!("\nWrote {}", out_path.display());
    } else {
        print!("{}", output);
        eprintln!("\n(dry-run: pass --write to overwrite models.rs)");
    }

    Ok(())
}
