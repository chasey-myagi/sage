//! Simple stream-option helpers — Rust port of pi-mono `providers/simple-options.ts`.
//!
//! Provides helpers for building [`crate::registry::StreamOptions`] from
//! simplified caller-supplied options, and for adjusting token budgets when
//! extended thinking / reasoning is enabled.

use crate::registry::{CacheRetention, StreamOptions};
use crate::types::{Model, ReasoningLevel};

// ============================================================================
// ThinkingBudgets
// ============================================================================

/// Per-level thinking token budgets (mirrors pi-mono `ThinkingBudgets`).
///
/// All fields are optional; missing levels fall back to the hard-coded
/// defaults in [`adjust_max_tokens_for_thinking`].
#[derive(Debug, Clone, Default)]
pub struct ThinkingBudgets {
    pub minimal: Option<u32>,
    pub low: Option<u32>,
    pub medium: Option<u32>,
    pub high: Option<u32>,
}

// ============================================================================
// SimpleStreamOptions
// ============================================================================

/// Simplified caller-supplied options (subset of [`StreamOptions`]).
///
/// Mirrors pi-mono's `SimpleStreamOptions` type: callers fill in only what
/// they need; defaults are filled in by [`build_base_options`].
#[derive(Debug, Clone, Default)]
pub struct SimpleStreamOptions {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub api_key: Option<String>,
    pub cache_retention: Option<CacheRetention>,
    pub session_id: Option<String>,
    pub headers: Vec<(String, String)>,
    pub max_retry_delay_ms: Option<u64>,
}

// ============================================================================
// build_base_options
// ============================================================================

/// Build a [`StreamOptions`] from simplified caller-supplied options and the
/// model's declared `max_tokens` cap.
///
/// Mirrors pi-mono's `buildBaseOptions`:
/// - `max_tokens` defaults to `min(model.max_tokens, 32000)` if not supplied.
/// - All other fields are taken directly from `options` when present.
pub fn build_base_options(model: &Model, options: Option<&SimpleStreamOptions>) -> StreamOptions {
    let max_tokens = options
        .and_then(|o| o.max_tokens)
        .unwrap_or_else(|| model.max_tokens.min(32000));

    StreamOptions {
        temperature: options.and_then(|o| o.temperature),
        max_tokens: Some(max_tokens),
        api_key: options.and_then(|o| o.api_key.clone()),
        cache_retention: options.and_then(|o| o.cache_retention),
        session_id: options.and_then(|o| o.session_id.clone()),
        headers: options.map(|o| o.headers.clone()).unwrap_or_default(),
        ..StreamOptions::default()
    }
}

// ============================================================================
// clamp_reasoning
// ============================================================================

/// Clamp a [`ReasoningLevel`] to at most `High` (i.e. map `XHigh` → `High`).
///
/// Some providers (e.g. OpenAI Responses via `adjustMaxTokensForThinking`)
/// don't support `xhigh`; this helper maps it down to the next supported level.
///
/// Mirrors pi-mono's `clampReasoning`.
pub fn clamp_reasoning(effort: ReasoningLevel) -> ReasoningLevel {
    if effort == ReasoningLevel::XHigh {
        ReasoningLevel::High
    } else {
        effort
    }
}

// ============================================================================
// adjust_max_tokens_for_thinking
// ============================================================================

/// Adjust `max_tokens` and compute a `thinking_budget` for reasoning-enabled
/// models.
///
/// Returns `(max_tokens, thinking_budget)`:
/// - `max_tokens` is clamped to `model.max_tokens`.
/// - `thinking_budget` is at most `max_tokens - 1024` (minimum 1 k output
///   tokens must remain).
///
/// Mirrors pi-mono's `adjustMaxTokensForThinking`.
pub fn adjust_max_tokens_for_thinking(
    base_max_tokens: u32,
    model_max_tokens: u32,
    reasoning_level: ReasoningLevel,
    custom_budgets: Option<&ThinkingBudgets>,
) -> (u32, u32) {
    // Default per-level budgets (tokens).
    let default_minimal: u32 = 1024;
    let default_low: u32 = 2048;
    let default_medium: u32 = 8192;
    let default_high: u32 = 16384;

    let level = clamp_reasoning(reasoning_level);

    let thinking_budget_raw = match level {
        ReasoningLevel::Minimal => custom_budgets
            .and_then(|b| b.minimal)
            .unwrap_or(default_minimal),
        ReasoningLevel::Low => custom_budgets.and_then(|b| b.low).unwrap_or(default_low),
        ReasoningLevel::Medium => custom_budgets
            .and_then(|b| b.medium)
            .unwrap_or(default_medium),
        ReasoningLevel::High | ReasoningLevel::XHigh => {
            custom_budgets.and_then(|b| b.high).unwrap_or(default_high)
        }
    };

    let min_output_tokens: u32 = 1024;
    let max_tokens = (base_max_tokens + thinking_budget_raw).min(model_max_tokens);

    let thinking_budget = if max_tokens <= thinking_budget_raw {
        max_tokens.saturating_sub(min_output_tokens)
    } else {
        thinking_budget_raw
    };

    (max_tokens, thinking_budget)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{InputType, ModelCost};

    fn test_model(max_tokens: u32) -> Model {
        Model {
            id: "test-model".into(),
            name: "Test Model".into(),
            api: "openai-completions".into(),
            provider: "openai".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens,
            context_window: 128000,
            cost: ModelCost {
                input_per_million: 1.0,
                output_per_million: 1.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        }
    }

    // ── build_base_options ────────────────────────────────────────────────────

    #[test]
    fn test_build_base_options_defaults() {
        let model = test_model(16384);
        let opts = build_base_options(&model, None);
        // max_tokens defaults to min(model.max_tokens, 32000) = 16384
        assert_eq!(opts.max_tokens, Some(16384));
        assert!(opts.api_key.is_none());
        assert!(opts.temperature.is_none());
    }

    #[test]
    fn test_build_base_options_caps_at_32000() {
        let model = test_model(100000);
        let opts = build_base_options(&model, None);
        // model.max_tokens > 32000, so cap at 32000
        assert_eq!(opts.max_tokens, Some(32000));
    }

    #[test]
    fn test_build_base_options_respects_caller_max_tokens() {
        let model = test_model(100000);
        let simple = SimpleStreamOptions {
            max_tokens: Some(4096),
            ..Default::default()
        };
        let opts = build_base_options(&model, Some(&simple));
        assert_eq!(opts.max_tokens, Some(4096));
    }

    #[test]
    fn test_build_base_options_propagates_api_key() {
        let model = test_model(4096);
        let simple = SimpleStreamOptions {
            api_key: Some("sk-test".into()),
            ..Default::default()
        };
        let opts = build_base_options(&model, Some(&simple));
        assert_eq!(opts.api_key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn test_build_base_options_propagates_temperature() {
        let model = test_model(4096);
        let simple = SimpleStreamOptions {
            temperature: Some(0.7),
            ..Default::default()
        };
        let opts = build_base_options(&model, Some(&simple));
        assert!((opts.temperature.unwrap() - 0.7).abs() < 1e-6);
    }

    // ── clamp_reasoning ───────────────────────────────────────────────────────

    #[test]
    fn test_clamp_reasoning_xhigh_becomes_high() {
        assert_eq!(clamp_reasoning(ReasoningLevel::XHigh), ReasoningLevel::High);
    }

    #[test]
    fn test_clamp_reasoning_other_levels_unchanged() {
        for level in [
            ReasoningLevel::Minimal,
            ReasoningLevel::Low,
            ReasoningLevel::Medium,
            ReasoningLevel::High,
        ] {
            assert_eq!(clamp_reasoning(level), level);
        }
    }

    // ── adjust_max_tokens_for_thinking ────────────────────────────────────────

    #[test]
    fn test_adjust_max_tokens_basic_medium() {
        // base=4096, model_max=100000, level=Medium (budget=8192)
        // max_tokens = min(4096 + 8192, 100000) = 12288
        // thinking_budget = 8192 (max_tokens > budget)
        let (max_tokens, budget) =
            adjust_max_tokens_for_thinking(4096, 100000, ReasoningLevel::Medium, None);
        assert_eq!(max_tokens, 12288);
        assert_eq!(budget, 8192);
    }

    #[test]
    fn test_adjust_max_tokens_clamps_to_model_max() {
        // base=30000, model_max=32000, level=High (budget=16384)
        // max_tokens = min(30000 + 16384, 32000) = 32000
        // thinking_budget = 16384 (max_tokens > budget)
        let (max_tokens, budget) =
            adjust_max_tokens_for_thinking(30000, 32000, ReasoningLevel::High, None);
        assert_eq!(max_tokens, 32000);
        assert_eq!(budget, 16384);
    }

    #[test]
    fn test_adjust_max_tokens_reduces_budget_when_constrained() {
        // base=0, model_max=1500, level=Minimal (budget=1024)
        // max_tokens = min(0 + 1024, 1500) = 1024
        // max_tokens (1024) <= budget (1024), so budget = max(0, 1024 - 1024) = 0
        let (max_tokens, budget) =
            adjust_max_tokens_for_thinking(0, 1500, ReasoningLevel::Minimal, None);
        assert_eq!(max_tokens, 1024);
        assert_eq!(budget, 0);
    }

    #[test]
    fn test_adjust_max_tokens_custom_budgets() {
        let custom = ThinkingBudgets {
            medium: Some(4000),
            ..Default::default()
        };
        let (max_tokens, budget) =
            adjust_max_tokens_for_thinking(8192, 100000, ReasoningLevel::Medium, Some(&custom));
        // budget = 4000, max_tokens = 8192 + 4000 = 12192
        assert_eq!(max_tokens, 12192);
        assert_eq!(budget, 4000);
    }

    #[test]
    fn test_adjust_max_tokens_xhigh_uses_high_budget() {
        // XHigh gets clamped to High (budget=16384)
        let (max_tokens_xhigh, budget_xhigh) =
            adjust_max_tokens_for_thinking(4096, 100000, ReasoningLevel::XHigh, None);
        let (max_tokens_high, budget_high) =
            adjust_max_tokens_for_thinking(4096, 100000, ReasoningLevel::High, None);
        assert_eq!(max_tokens_xhigh, max_tokens_high);
        assert_eq!(budget_xhigh, budget_high);
    }
}
