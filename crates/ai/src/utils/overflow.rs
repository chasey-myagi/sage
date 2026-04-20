//! Context-overflow detection — 1:1 port of `packages/ai/src/utils/overflow.ts`.
//!
//! Every regex pattern, every case branch, matches pi-mono exactly.

use regex::Regex;
use std::sync::LazyLock;

use crate::types::{StopReason, Usage};

/// Regex patterns that match context-overflow error messages across providers.
///
/// Each entry comments the provider + an example message it matches. Any edit
/// here should mirror the corresponding edit in
/// `packages/ai/src/utils/overflow.ts`.
static OVERFLOW_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    // NB: all patterns are case-insensitive (`(?i)`) to match pi-mono's `/i` flag.
    [
        r"(?i)prompt is too long",                                // Anthropic
        r"(?i)input is too long for requested model",             // Amazon Bedrock
        r"(?i)exceeds the context window",                        // OpenAI
        r"(?i)input token count.*exceeds the maximum",            // Google Gemini
        r"(?i)maximum prompt length is \d+",                      // xAI (Grok)
        r"(?i)reduce the length of the messages",                 // Groq
        r"(?i)maximum context length is \d+ tokens",              // OpenRouter
        r"(?i)exceeds the limit of \d+",                          // GitHub Copilot
        r"(?i)exceeds the available context size",                // llama.cpp
        r"(?i)greater than the context length",                   // LM Studio
        r"(?i)context window exceeds limit",                      // MiniMax
        r"(?i)exceeded model token limit",                        // Kimi For Coding
        r"(?i)too large for model with \d+ maximum context length", // Mistral
        r"(?i)model_context_window_exceeded",                     // z.ai
        r"(?i)context[_ ]length[_ ]exceeded",                     // generic fallback
        r"(?i)too many tokens",                                   // generic fallback
        r"(?i)token limit exceeded",                              // generic fallback
    ]
    .iter()
    .map(|p| Regex::new(p).expect("overflow pattern must compile"))
    .collect()
});

/// Matches "400/413 status code (no body)" — Cerebras' overflow signal.
static CEREBRAS_NO_BODY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^4(00|13)\s*(status code)?\s*\(no body\)")
        .expect("cerebras pattern must compile")
});

/// Minimal projection of `AssistantMessage` used by [`is_context_overflow`].
///
/// sage doesn't have pi-mono's full `AssistantMessage` type (see the module-
/// level comment in `utils/mod.rs`), so we take the three fields that matter
/// for overflow detection directly. Callers typically build this from the
/// event stream they just drained.
#[derive(Debug, Clone)]
pub struct OverflowCheck<'a> {
    pub stop_reason: StopReason,
    pub error_message: Option<&'a str>,
    pub usage: &'a Usage,
}

/// Does this assistant response represent a context-window overflow?
///
/// Mirrors `isContextOverflow` in pi-mono's `overflow.ts`:
///
/// 1. **Error-based overflow**: `stop_reason == Error` and the error message
///    matches any provider pattern (or the Cerebras "400/413 (no body)" form).
/// 2. **Silent overflow** (z.ai style): `stop_reason == Stop` and
///    `usage.input + usage.cache_read > context_window`. Only checked when
///    `context_window` is supplied.
pub fn is_context_overflow(msg: &OverflowCheck<'_>, context_window: Option<u64>) -> bool {
    // Case 1: error-based overflow
    if msg.stop_reason == StopReason::Error {
        if let Some(err) = msg.error_message {
            if OVERFLOW_PATTERNS.iter().any(|p| p.is_match(err)) {
                return true;
            }
            if CEREBRAS_NO_BODY.is_match(err) {
                return true;
            }
        }
    }

    // Case 2: silent overflow (successful stop, usage > window)
    if let Some(window) = context_window {
        if msg.stop_reason == StopReason::Stop {
            let input_tokens = msg.usage.input + msg.usage.cache_read;
            if input_tokens > window {
                return true;
            }
        }
    }

    false
}

/// Return a clone of the overflow patterns — mirrors `getOverflowPatterns`.
/// Intended for tests only.
pub fn get_overflow_patterns() -> Vec<Regex> {
    OVERFLOW_PATTERNS.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Cost, StopReason, Usage};

    fn usage(input: u64, cache_read: u64) -> Usage {
        Usage {
            input,
            output: 0,
            cache_read,
            cache_write: 0,
            total_tokens: input + cache_read,
            cost: Cost::default(),
        }
    }

    fn check<'a>(
        stop: StopReason,
        err: Option<&'a str>,
        u: &'a Usage,
    ) -> OverflowCheck<'a> {
        OverflowCheck {
            stop_reason: stop,
            error_message: err,
            usage: u,
        }
    }

    // --- error-based overflow -------------------------------------------

    #[test]
    fn matches_anthropic_prompt_too_long() {
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("prompt is too long: 213462 tokens > 200000 maximum"),
            &u,
        );
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn matches_openai_context_window() {
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("Your input exceeds the context window of this model"),
            &u,
        );
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn matches_google_gemini() {
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("The input token count (1196265) exceeds the maximum number of tokens allowed (1048575)"),
            &u,
        );
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn matches_xai_grok() {
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("This model's maximum prompt length is 131072 but the request contains 537812 tokens"),
            &u,
        );
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn matches_groq_reduce_length() {
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("Please reduce the length of the messages or completion"),
            &u,
        );
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn matches_openrouter() {
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("This endpoint's maximum context length is 8192 tokens. However, you requested about 10000 tokens"),
            &u,
        );
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn matches_cerebras_no_body() {
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("400 status code (no body)"),
            &u,
        );
        assert!(is_context_overflow(&msg, None));

        let msg2 = check(
            StopReason::Error,
            Some("413 (no body)"),
            &u,
        );
        assert!(is_context_overflow(&msg2, None));
    }

    #[test]
    fn matches_bedrock() {
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("Input is too long for requested model."),
            &u,
        );
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn matches_mistral() {
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("Prompt contains 40000 tokens ... too large for model with 32768 maximum context length"),
            &u,
        );
        assert!(is_context_overflow(&msg, None));
    }

    // --- silent overflow ------------------------------------------------

    #[test]
    fn detects_zai_silent_overflow() {
        let u = usage(8000, 2000); // 10000 total, window 9000
        let msg = check(StopReason::Stop, None, &u);
        assert!(is_context_overflow(&msg, Some(9000)));
    }

    #[test]
    fn no_silent_overflow_when_within_window() {
        let u = usage(5000, 1000); // 6000 total, window 9000
        let msg = check(StopReason::Stop, None, &u);
        assert!(!is_context_overflow(&msg, Some(9000)));
    }

    #[test]
    fn silent_overflow_ignored_without_window() {
        let u = usage(100_000, 0);
        let msg = check(StopReason::Stop, None, &u);
        assert!(!is_context_overflow(&msg, None));
    }

    // --- non-overflow errors --------------------------------------------

    #[test]
    fn unrelated_error_not_overflow() {
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("Rate limit exceeded, please retry later"),
            &u,
        );
        assert!(!is_context_overflow(&msg, None));
    }

    #[test]
    fn error_429_rate_limit_not_overflow() {
        // Explicitly: 429 != 400/413
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("429 (no body)"),
            &u,
        );
        assert!(!is_context_overflow(&msg, None));
    }

    #[test]
    fn get_overflow_patterns_not_empty() {
        let patterns = get_overflow_patterns();
        assert!(!patterns.is_empty());
        // ensure the first pattern matches Anthropic's format (sanity check)
        assert!(patterns[0].is_match("prompt is too long"));
    }

    // --- case-insensitivity (pi-mono uses /i flag) -------------------------

    #[test]
    fn patterns_are_case_insensitive() {
        let u = usage(0, 0);
        // Anthropic uppercase
        let msg = check(StopReason::Error, Some("PROMPT IS TOO LONG: 1000 > 200000"), &u);
        assert!(is_context_overflow(&msg, None));
        // OpenAI mixed case
        let msg2 = check(StopReason::Error, Some("Your Input Exceeds The Context Window"), &u);
        assert!(is_context_overflow(&msg2, None));
    }

    // --- stop + correct usage → no overflow --------------------------------

    #[test]
    fn stop_with_exact_window_is_not_overflow() {
        let u = usage(9000, 0); // input == contextWindow
        let msg = check(StopReason::Stop, None, &u);
        // Strictly greater, so equal should NOT be overflow
        assert!(!is_context_overflow(&msg, Some(9000)));
    }

    // --- error without message → no overflow --------------------------------

    #[test]
    fn error_with_no_message_is_not_overflow() {
        let u = usage(0, 0);
        let msg = check(StopReason::Error, None, &u);
        assert!(!is_context_overflow(&msg, None));
    }

    // --- tool_use stop reason -----------------------------------------------

    #[test]
    fn tool_use_stop_reason_not_overflow_even_with_large_usage() {
        let u = usage(100_000, 0);
        let msg = check(StopReason::ToolUse, None, &u);
        // only StopReason::Stop triggers silent-overflow check
        assert!(!is_context_overflow(&msg, Some(50_000)));
    }

    // --- additional provider patterns (pi-mono 1:1 mirror) ------------------

    #[test]
    fn matches_github_copilot_exceeds_limit() {
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("prompt token count of 150000 exceeds the limit of 128000"),
            &u,
        );
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn matches_llama_cpp_exceeds_available_context_size() {
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("the request exceeds the available context size, try increasing it"),
            &u,
        );
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn matches_lm_studio_greater_than_context_length() {
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("tokens to keep from the initial prompt is greater than the context length"),
            &u,
        );
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn matches_minimax_context_window_exceeds_limit() {
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("invalid params, context window exceeds limit"),
            &u,
        );
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn matches_kimi_exceeded_model_token_limit() {
        let u = usage(0, 0);
        let msg = check(
            StopReason::Error,
            Some("Your request exceeded model token limit: 128000 (requested: 200000)"),
            &u,
        );
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn matches_generic_context_length_exceeded() {
        let u = usage(0, 0);
        let msg = check(StopReason::Error, Some("context length exceeded"), &u);
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn matches_generic_too_many_tokens() {
        let u = usage(0, 0);
        let msg = check(StopReason::Error, Some("too many tokens in request"), &u);
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn matches_generic_token_limit_exceeded() {
        let u = usage(0, 0);
        let msg = check(StopReason::Error, Some("token limit exceeded"), &u);
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn matches_zai_model_context_window_exceeded() {
        let u = usage(0, 0);
        let msg = check(StopReason::Error, Some("model_context_window_exceeded"), &u);
        assert!(is_context_overflow(&msg, None));
    }

    // --- Cerebras 400/413 edge cases ----------------------------------------

    #[test]
    fn cerebras_400_no_body() {
        let u = usage(0, 0);
        let msg = check(StopReason::Error, Some("400 (no body)"), &u);
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn cerebras_413_no_body_no_status_code_text() {
        let u = usage(0, 0);
        let msg = check(StopReason::Error, Some("413 (no body)"), &u);
        assert!(is_context_overflow(&msg, None));
    }

    #[test]
    fn non_cerebras_500_no_body_not_overflow() {
        let u = usage(0, 0);
        let msg = check(StopReason::Error, Some("500 (no body)"), &u);
        assert!(!is_context_overflow(&msg, None));
    }

    // --- silent overflow boundary -------------------------------------------

    #[test]
    fn silent_overflow_exactly_one_over_window() {
        let u = usage(9001, 0);
        let msg = check(StopReason::Stop, None, &u);
        assert!(is_context_overflow(&msg, Some(9000)));
    }

    #[test]
    fn silent_overflow_with_cache_read_contributes() {
        // input + cache_read together exceed window
        let u = usage(5000, 5001); // 10001 total > 10000
        let msg = check(StopReason::Stop, None, &u);
        assert!(is_context_overflow(&msg, Some(10_000)));
    }

    #[test]
    fn silent_overflow_with_context_window_zero_never_triggered() {
        // context_window=0 would mean everything overflows; we pass None for no check
        let u = usage(1, 0);
        let msg = check(StopReason::Stop, None, &u);
        // No context_window provided
        assert!(!is_context_overflow(&msg, None));
    }

    // --- get_overflow_patterns mirrors pi-mono count -------------------------

    #[test]
    fn get_overflow_patterns_matches_expected_count() {
        // pi-mono defines 17 patterns (see overflow.ts OVERFLOW_PATTERNS array)
        let patterns = get_overflow_patterns();
        assert_eq!(patterns.len(), 17, "pattern count should match pi-mono");
    }
}
