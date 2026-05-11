//! Model resolution, scoping, and initial selection.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/model-resolver.ts`.

use std::collections::HashMap;

// ============================================================================
// Default model IDs per provider
// ============================================================================

/// Default model ID for each known provider.
/// Mirrors `defaultModelPerProvider` from TypeScript.
pub fn default_model_per_provider() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("amazon-bedrock", "us.anthropic.claude-opus-4-6-v1");
    m.insert("anthropic", "claude-opus-4-6");
    m.insert("openai", "gpt-5.4");
    m.insert("azure-openai-responses", "gpt-5.2");
    m.insert("openai-codex", "gpt-5.4");
    m.insert("google", "gemini-2.5-pro");
    m.insert("google-gemini-cli", "gemini-2.5-pro");
    m.insert("google-antigravity", "gemini-3.1-pro-high");
    m.insert("google-vertex", "gemini-3-pro-preview");
    m.insert("github-copilot", "gpt-4o");
    m.insert("openrouter", "openai/gpt-5.1-codex");
    m.insert("vercel-ai-gateway", "anthropic/claude-opus-4-6");
    m.insert("xai", "grok-4-fast-non-reasoning");
    m.insert("groq", "openai/gpt-oss-120b");
    m.insert("cerebras", "zai-glm-4.7");
    m.insert("zai", "glm-5");
    m.insert("mistral", "devstral-medium-latest");
    m.insert("minimax", "MiniMax-M2.7");
    m.insert("minimax-cn", "MiniMax-M2.7");
    m.insert("huggingface", "moonshotai/Kimi-K2.5");
    m.insert("opencode", "claude-opus-4-6");
    m.insert("opencode-go", "kimi-k2.5");
    m.insert("kimi-coding", "kimi-k2-thinking");
    m
}

// ============================================================================
// Model representation (simplified — full model is in model_registry.rs)
// ============================================================================

#[derive(Debug, Clone)]
pub struct ModelRef {
    pub provider: String,
    pub id: String,
    pub name: Option<String>,
    pub reasoning: bool,
    pub context_window: u64,
    pub max_tokens: u64,
}

impl ModelRef {
    pub fn full_id(&self) -> String {
        format!("{}/{}", self.provider, self.id)
    }
}

// ============================================================================
// Scoped model
// ============================================================================

#[derive(Debug, Clone)]
pub struct ScopedModel {
    pub model: ModelRef,
    /// Thinking level explicitly specified in pattern (e.g. `"model:high"`).
    pub thinking_level: Option<String>,
}

// ============================================================================
// Alias detection
// ============================================================================

/// Whether a model ID looks like an alias (no date suffix).
/// Dates are typically `-YYYYMMDD`.
fn is_alias(id: &str) -> bool {
    if id.ends_with("-latest") {
        return true;
    }
    // Check if ends with -YYYYMMDD (8 digits)
    let parts: Vec<&str> = id.rsplitn(2, '-').collect();
    if parts.len() == 2 && parts[0].len() == 8 && parts[0].chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    true
}

// ============================================================================
// Valid thinking levels
// ============================================================================

pub fn is_valid_thinking_level(s: &str) -> bool {
    matches!(s, "off" | "low" | "medium" | "high" | "max")
}

// ============================================================================
// Exact model reference match
// ============================================================================

/// Find an exact model reference match.
/// Supports `provider/modelId` or bare `modelId`.
///
/// Mirrors `findExactModelReferenceMatch()` from TypeScript.
pub fn find_exact_model_reference_match<'a>(
    reference: &str,
    available: &'a [ModelRef],
) -> Option<&'a ModelRef> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_lowercase();

    // Try canonical provider/id match
    let canonical: Vec<&ModelRef> = available
        .iter()
        .filter(|m| m.full_id().to_lowercase() == lower)
        .collect();
    if canonical.len() == 1 {
        return Some(canonical[0]);
    }
    if canonical.len() > 1 {
        return None; // Ambiguous
    }

    // Try provider + id split on first slash
    if let Some(slash) = trimmed.find('/') {
        let provider = &trimmed[..slash];
        let model_id = &trimmed[slash + 1..];
        if !provider.is_empty() && !model_id.is_empty() {
            let matches: Vec<&ModelRef> = available
                .iter()
                .filter(|m| {
                    m.provider.to_lowercase() == provider.to_lowercase()
                        && m.id.to_lowercase() == model_id.to_lowercase()
                })
                .collect();
            if matches.len() == 1 {
                return Some(matches[0]);
            }
            if matches.len() > 1 {
                return None;
            }
        }
    }

    // Try bare id match
    let id_matches: Vec<&ModelRef> = available
        .iter()
        .filter(|m| m.id.to_lowercase() == lower)
        .collect();
    if id_matches.len() == 1 {
        return Some(id_matches[0]);
    }

    None
}

// ============================================================================
// Pattern matching
// ============================================================================

/// Try to match a pattern to a model (exact then partial).
fn try_match_model<'a>(pattern: &str, available: &'a [ModelRef]) -> Option<&'a ModelRef> {
    if let Some(m) = find_exact_model_reference_match(pattern, available) {
        return Some(m);
    }

    // Partial match by id or name
    let lower = pattern.to_lowercase();
    let matches: Vec<&ModelRef> = available
        .iter()
        .filter(|m| {
            m.id.to_lowercase().contains(&lower)
                || m.name
                    .as_ref()
                    .is_some_and(|n| n.to_lowercase().contains(&lower))
        })
        .collect();

    if matches.is_empty() {
        return None;
    }

    // Prefer aliases; within same type, sort descending by id
    let mut aliases: Vec<&ModelRef> = matches
        .iter()
        .copied()
        .filter(|m| is_alias(&m.id))
        .collect();
    let mut dated: Vec<&ModelRef> = matches
        .iter()
        .copied()
        .filter(|m| !is_alias(&m.id))
        .collect();

    if !aliases.is_empty() {
        aliases.sort_by(|a, b| b.id.cmp(&a.id));
        return Some(aliases[0]);
    }
    dated.sort_by(|a, b| b.id.cmp(&a.id));
    Some(dated[0])
}

// ============================================================================
// Parse model pattern (supports `:thinking_level` suffix)
// ============================================================================

#[derive(Debug, Clone)]
pub struct ParsedModelResult<'a> {
    pub model: Option<&'a ModelRef>,
    pub thinking_level: Option<String>,
    pub warning: Option<String>,
}

/// Parse a pattern to extract model and optional thinking level.
///
/// Algorithm (mirrors TypeScript):
/// 1. Try full pattern as model → return it
/// 2. If no match and has colons, split on last colon:
///    - If suffix is a valid thinking level → recurse on prefix
///    - Otherwise → warn and recurse on prefix (if `allow_invalid_fallback`)
pub fn parse_model_pattern<'a>(
    pattern: &str,
    available: &'a [ModelRef],
    allow_invalid_thinking_level_fallback: bool,
) -> ParsedModelResult<'a> {
    if let Some(m) = try_match_model(pattern, available) {
        return ParsedModelResult {
            model: Some(m),
            thinking_level: None,
            warning: None,
        };
    }

    let last_colon = match pattern.rfind(':') {
        Some(i) => i,
        None => {
            return ParsedModelResult {
                model: None,
                thinking_level: None,
                warning: None,
            };
        }
    };

    let prefix = &pattern[..last_colon];
    let suffix = &pattern[last_colon + 1..];

    if is_valid_thinking_level(suffix) {
        let inner = parse_model_pattern(prefix, available, allow_invalid_thinking_level_fallback);
        if inner.model.is_some() {
            return ParsedModelResult {
                model: inner.model,
                thinking_level: if inner.warning.is_none() {
                    Some(suffix.to_string())
                } else {
                    None
                },
                warning: inner.warning,
            };
        }
        return inner;
    }

    // Invalid suffix
    if !allow_invalid_thinking_level_fallback {
        return ParsedModelResult {
            model: None,
            thinking_level: None,
            warning: None,
        };
    }

    let inner = parse_model_pattern(prefix, available, allow_invalid_thinking_level_fallback);
    if inner.model.is_some() {
        return ParsedModelResult {
            model: inner.model,
            thinking_level: None,
            warning: Some(format!(
                "Invalid thinking level \"{suffix}\" in pattern \"{pattern}\". Using default instead."
            )),
        };
    }
    inner
}

// ============================================================================
// CLI model resolution
// ============================================================================

#[derive(Debug, Clone)]
pub struct ResolveCliModelResult<'a> {
    pub model: Option<&'a ModelRef>,
    pub thinking_level: Option<String>,
    pub warning: Option<String>,
    pub error: Option<String>,
}

/// Resolve a single model from CLI flags.
///
/// Mirrors `resolveCliModel()` from TypeScript.
pub fn resolve_cli_model<'a>(
    cli_provider: Option<&str>,
    cli_model: Option<&str>,
    available: &'a [ModelRef],
) -> ResolveCliModelResult<'a> {
    let cli_model = match cli_model {
        Some(m) => m,
        None => {
            return ResolveCliModelResult {
                model: None,
                thinking_level: None,
                warning: None,
                error: None,
            };
        }
    };

    if available.is_empty() {
        return ResolveCliModelResult {
            model: None,
            thinking_level: None,
            warning: None,
            error: Some(
                "No models available. Check your installation or add models to models.json."
                    .to_string(),
            ),
        };
    }

    // Build provider map (case-insensitive)
    let mut provider_map: HashMap<String, String> = HashMap::new();
    for m in available {
        provider_map
            .entry(m.provider.to_lowercase())
            .or_insert_with(|| m.provider.clone());
    }

    let mut provider = cli_provider.and_then(|p| provider_map.get(&p.to_lowercase()).cloned());

    if let Some(cli_p) = cli_provider
        && provider.is_none()
    {
        return ResolveCliModelResult {
            model: None,
            thinking_level: None,
            warning: None,
            error: Some(format!(
                "Unknown provider \"{cli_p}\". Use --list-models to see available providers/models."
            )),
        };
    }

    // Try to interpret "provider/model" format
    let mut pattern = cli_model.to_string();
    let mut inferred_provider = false;

    if provider.is_none()
        && let Some(slash) = cli_model.find('/')
    {
        let maybe_provider = &cli_model[..slash];
        if let Some(canonical) = provider_map.get(&maybe_provider.to_lowercase()).cloned() {
            provider = Some(canonical);
            pattern = cli_model[slash + 1..].to_string();
            inferred_provider = true;
        }
    }

    // Try exact match on full input without provider inference
    if provider.is_none() {
        let lower = cli_model.to_lowercase();
        if let Some(exact) = available
            .iter()
            .find(|m| m.id.to_lowercase() == lower || m.full_id().to_lowercase() == lower)
        {
            return ResolveCliModelResult {
                model: Some(exact),
                thinking_level: None,
                warning: None,
                error: None,
            };
        }
    }

    // Strip duplicate provider prefix if both --provider and provider/model given
    if cli_provider.is_some()
        && let Some(ref p) = provider
    {
        let prefix = format!("{}/", p);
        if cli_model.to_lowercase().starts_with(&prefix.to_lowercase()) {
            pattern = cli_model[prefix.len()..].to_string();
        }
    }

    // Collect owned clones so we can pass as &[ModelRef]
    let candidates: Vec<ModelRef> = if let Some(ref p) = provider {
        available
            .iter()
            .filter(|m| &m.provider == p)
            .cloned()
            .collect()
    } else {
        available.to_vec()
    };

    let result = parse_model_pattern(&pattern, &candidates, false);

    if let Some(matched) = result.model {
        // Re-find the same model in `available` so the returned reference
        // has the lifetime `'a` (tied to `available`) rather than `candidates`.
        let model_in_available = available
            .iter()
            .find(|m| m.id == matched.id && m.provider == matched.provider);
        return ResolveCliModelResult {
            model: model_in_available,
            thinking_level: result.thinking_level,
            warning: result.warning,
            error: None,
        };
    }

    // If inferred provider but no match, try full input
    if inferred_provider {
        let lower = cli_model.to_lowercase();
        if let Some(exact) = available
            .iter()
            .find(|m| m.id.to_lowercase() == lower || m.full_id().to_lowercase() == lower)
        {
            return ResolveCliModelResult {
                model: Some(exact),
                thinking_level: None,
                warning: None,
                error: None,
            };
        }
        let fallback = parse_model_pattern(cli_model, available, false);
        if fallback.model.is_some() {
            return ResolveCliModelResult {
                model: fallback.model,
                thinking_level: fallback.thinking_level,
                warning: fallback.warning,
                error: None,
            };
        }
    }

    let display = provider
        .as_deref()
        .map(|p| format!("{p}/{pattern}"))
        .unwrap_or_else(|| cli_model.to_string());

    ResolveCliModelResult {
        model: None,
        thinking_level: None,
        warning: None,
        error: Some(format!(
            "Model \"{display}\" not found. Use --list-models to see available models."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_model(provider: &str, id: &str) -> ModelRef {
        ModelRef {
            provider: provider.to_string(),
            id: id.to_string(),
            name: Some(format!("{provider} {id}")),
            reasoning: false,
            context_window: 128_000,
            max_tokens: 4096,
        }
    }

    fn mock_models() -> Vec<ModelRef> {
        vec![
            make_model("anthropic", "claude-sonnet-4-5"),
            make_model("openai", "gpt-4o"),
            // OpenRouter models with colons in IDs
            make_model("openrouter", "qwen/qwen3-coder:exacto"),
            make_model("openrouter", "openai/gpt-4o:extended"),
        ]
    }

    // ---- find_exact_model_reference_match ----

    #[test]
    fn exact_match_by_full_id() {
        let models = vec![
            make_model("anthropic", "claude-opus-4-6"),
            make_model("openai", "gpt-4o"),
        ];
        let found = find_exact_model_reference_match("anthropic/claude-opus-4-6", &models);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "claude-opus-4-6");
    }

    // ---- is_alias ----

    #[test]
    fn is_alias_detects_dated_version() {
        assert!(!is_alias("claude-sonnet-4-5-20250929"));
        assert!(is_alias("claude-sonnet-4-5"));
        assert!(is_alias("claude-opus-4-6-latest"));
    }

    // ---- parseModelPattern: simple patterns without colons ----

    #[test]
    fn parse_exact_match_returns_model_with_no_thinking_level() {
        let models = mock_models();
        let result = parse_model_pattern("claude-sonnet-4-5", &models, true);
        assert_eq!(
            result.model.map(|m| m.id.as_str()),
            Some("claude-sonnet-4-5")
        );
        assert!(result.thinking_level.is_none());
        assert!(result.warning.is_none());
    }

    #[test]
    fn parse_partial_match_returns_best_model() {
        let models = mock_models();
        let result = parse_model_pattern("sonnet", &models, true);
        assert_eq!(
            result.model.map(|m| m.id.as_str()),
            Some("claude-sonnet-4-5")
        );
        assert!(result.thinking_level.is_none());
        assert!(result.warning.is_none());
    }

    #[test]
    fn parse_no_match_returns_none() {
        let models = mock_models();
        let result = parse_model_pattern("nonexistent", &models, true);
        assert!(result.model.is_none());
        assert!(result.thinking_level.is_none());
    }

    // ---- parseModelPattern: patterns with valid thinking levels ----

    #[test]
    fn parse_model_pattern_with_thinking_level() {
        let models = vec![make_model("anthropic", "claude-opus-4-6")];
        let result = parse_model_pattern("claude-opus-4-6:high", &models, true);
        assert!(result.model.is_some());
        assert_eq!(result.thinking_level.as_deref(), Some("high"));
    }

    #[test]
    fn parse_sonnet_high_returns_sonnet_with_high_thinking() {
        let models = mock_models();
        let result = parse_model_pattern("sonnet:high", &models, true);
        assert_eq!(
            result.model.map(|m| m.id.as_str()),
            Some("claude-sonnet-4-5")
        );
        assert_eq!(result.thinking_level.as_deref(), Some("high"));
        assert!(result.warning.is_none());
    }

    #[test]
    fn parse_gpt4o_medium_returns_gpt4o_with_medium_thinking() {
        let models = mock_models();
        let result = parse_model_pattern("gpt-4o:medium", &models, true);
        assert_eq!(result.model.map(|m| m.id.as_str()), Some("gpt-4o"));
        assert_eq!(result.thinking_level.as_deref(), Some("medium"));
        assert!(result.warning.is_none());
    }

    #[test]
    fn parse_all_valid_thinking_levels() {
        let models = mock_models();
        for level in &["off", "low", "medium", "high", "max"] {
            let result = parse_model_pattern(&format!("sonnet:{level}"), &models, true);
            assert_eq!(
                result.model.map(|m| m.id.as_str()),
                Some("claude-sonnet-4-5")
            );
            assert_eq!(result.thinking_level.as_deref(), Some(*level));
            assert!(result.warning.is_none(), "level={level}");
        }
    }

    // ---- parseModelPattern: invalid thinking levels ----

    #[test]
    fn parse_invalid_thinking_level_returns_warning() {
        let models = mock_models();
        let result = parse_model_pattern("sonnet:random", &models, true);
        assert_eq!(
            result.model.map(|m| m.id.as_str()),
            Some("claude-sonnet-4-5")
        );
        assert!(result.thinking_level.is_none());
        let warning = result.warning.unwrap_or_default();
        assert!(
            warning.contains("Invalid thinking level"),
            "warning={warning}"
        );
        assert!(warning.contains("random"), "warning={warning}");
    }

    #[test]
    fn parse_gpt4o_invalid_level_returns_warning() {
        let models = mock_models();
        let result = parse_model_pattern("gpt-4o:invalid", &models, true);
        assert_eq!(result.model.map(|m| m.id.as_str()), Some("gpt-4o"));
        assert!(result.thinking_level.is_none());
        assert!(
            result
                .warning
                .as_deref()
                .unwrap_or("")
                .contains("Invalid thinking level")
        );
    }

    // ---- parseModelPattern: OpenRouter models with colons in IDs ----

    #[test]
    fn parse_openrouter_model_with_colon_in_id() {
        let models = mock_models();
        let result = parse_model_pattern("qwen/qwen3-coder:exacto", &models, true);
        assert_eq!(
            result.model.map(|m| m.id.as_str()),
            Some("qwen/qwen3-coder:exacto")
        );
        assert!(result.thinking_level.is_none());
        assert!(result.warning.is_none());
    }

    #[test]
    fn parse_openrouter_model_with_provider_prefix() {
        let models = mock_models();
        // Provider-prefixed form: "openrouter/qwen/qwen3-coder:exacto"
        let result = parse_model_pattern("openrouter/qwen/qwen3-coder:exacto", &models, true);
        assert_eq!(
            result.model.map(|m| m.id.as_str()),
            Some("qwen/qwen3-coder:exacto")
        );
        assert_eq!(
            result.model.map(|m| m.provider.as_str()),
            Some("openrouter")
        );
        assert!(result.thinking_level.is_none());
    }

    #[test]
    fn parse_openrouter_model_with_thinking_level() {
        let models = mock_models();
        let result = parse_model_pattern("qwen/qwen3-coder:exacto:high", &models, true);
        assert_eq!(
            result.model.map(|m| m.id.as_str()),
            Some("qwen/qwen3-coder:exacto")
        );
        assert_eq!(result.thinking_level.as_deref(), Some("high"));
        assert!(result.warning.is_none());
    }

    // ---- resolveCliModel ----

    #[test]
    fn resolve_cli_model_provider_slash_id() {
        let models = mock_models();
        let result = resolve_cli_model(None, Some("openai/gpt-4o"), &models);
        assert!(result.error.is_none());
        assert_eq!(result.model.map(|m| m.provider.as_str()), Some("openai"));
        assert_eq!(result.model.map(|m| m.id.as_str()), Some("gpt-4o"));
    }

    #[test]
    fn resolve_cli_model_fuzzy_within_provider() {
        let models = mock_models();
        let result = resolve_cli_model(Some("openai"), Some("4o"), &models);
        assert!(result.error.is_none());
        assert_eq!(result.model.map(|m| m.provider.as_str()), Some("openai"));
        assert_eq!(result.model.map(|m| m.id.as_str()), Some("gpt-4o"));
    }

    #[test]
    fn resolve_cli_model_with_thinking_suffix() {
        let models = mock_models();
        let result = resolve_cli_model(None, Some("sonnet:high"), &models);
        assert!(result.error.is_none());
        assert_eq!(
            result.model.map(|m| m.id.as_str()),
            Some("claude-sonnet-4-5")
        );
        assert_eq!(result.thinking_level.as_deref(), Some("high"));
    }

    #[test]
    fn resolve_cli_model_not_found_returns_error() {
        let models = mock_models();
        let result = resolve_cli_model(None, Some("nonexistent-model-xyz"), &models);
        assert!(result.error.is_some());
        assert!(result.model.is_none());
    }

    #[test]
    fn resolve_cli_model_unknown_provider_returns_error() {
        let models = mock_models();
        let result = resolve_cli_model(Some("unknown-provider"), Some("some-model"), &models);
        assert!(result.error.is_some());
        assert!(result.model.is_none());
    }

    #[test]
    fn resolve_cli_model_no_models_returns_error() {
        let result = resolve_cli_model(None, Some("any-model"), &[]);
        assert!(result.error.is_some());
    }

    // ---- default_model_per_provider ----

    #[test]
    fn default_model_per_provider_has_anthropic() {
        let defaults = default_model_per_provider();
        assert!(defaults.contains_key("anthropic"));
        assert!(defaults.contains_key("openai"));
        assert!(defaults.contains_key("google"));
    }
}
