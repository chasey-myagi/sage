//! Shared utilities for Google Generative AI and Google Vertex AI providers.
//!
//! Rust port of pi-mono `providers/google-shared.ts`.
//!
//! The heavy-weight message/tool conversion and SSE stream reading are already
//! implemented in [`super::google`] (`google.rs`) because that module was
//! ported first and serves as the single authoritative copy of those routines
//! (re-exported to `google_vertex.rs`).  This module focuses on the helpers
//! that the TypeScript shared module exposes but that were not yet surfaced in
//! the Rust code:
//!
//! * [`is_thinking_part`] — determines whether a streamed Gemini part is
//!   "thinking" content.
//! * [`retain_thought_signature`] — preserves the last non-empty thought
//!   signature during streaming.
//! * [`requires_tool_call_id`] — whether the model requires explicit tool-call
//!   IDs in function calls/responses.
//! * [`map_stop_reason`] / [`map_stop_reason_string`] — FinishReason → StopReason.
//!
//! Re-exports from `super::google` are provided for callers that want a single
//! import point.

use crate::types::StopReason;

// ============================================================================
// Thought-signature helpers
// ============================================================================

/// Determine whether a streamed Gemini `Part` should be treated as "thinking".
///
/// Protocol note (Gemini / Vertex AI thought signatures):
/// - `thought: true` is the definitive marker for thinking content.
/// - `thoughtSignature` can appear on **any** part type and does **not** imply
///   that the part itself is thinking content.
///
/// Mirrors pi-mono's `isThinkingPart`.
///
/// # Arguments
/// * `thought` — the value of the part's `thought` field, if present.
/// * `_thought_signature` — accepted but ignored; present for API compatibility.
pub fn is_thinking_part(thought: Option<bool>, _thought_signature: Option<&str>) -> bool {
    thought == Some(true)
}

/// Retain the last non-empty thought signature seen for the current block.
///
/// Some backends only send `thoughtSignature` on the first delta for a given
/// part/block; later deltas may omit it.  This helper keeps the previous
/// non-empty signature rather than overwriting it with `None`.
///
/// Note: this does **not** merge or move signatures across distinct response
/// parts — it only prevents a signature from being cleared within the same
/// streamed block.
///
/// Mirrors pi-mono's `retainThoughtSignature`.
pub fn retain_thought_signature<'a>(
    existing: Option<&'a str>,
    incoming: Option<&'a str>,
) -> Option<&'a str> {
    if let Some(s) = incoming {
        if !s.is_empty() {
            return Some(s);
        }
    }
    existing
}

/// Owned-string variant of [`retain_thought_signature`].
///
/// Useful when signatures are stored as `Option<String>` rather than `&str`.
pub fn retain_thought_signature_owned(
    existing: Option<String>,
    incoming: Option<String>,
) -> Option<String> {
    if let Some(ref s) = incoming {
        if !s.is_empty() {
            return incoming;
        }
    }
    existing
}

// ============================================================================
// Tool-call ID helpers
// ============================================================================

/// Whether a model requires explicit tool-call IDs in function calls/responses.
///
/// Models accessed via Google APIs that require this are:
/// - Claude models (provider prefix `claude-`).
/// - GPT-OSS models (`gpt-oss-` prefix).
///
/// Mirrors pi-mono's `requiresToolCallId`.
pub fn requires_tool_call_id(model_id: &str) -> bool {
    model_id.starts_with("claude-") || model_id.starts_with("gpt-oss-")
}

// ============================================================================
// Stop-reason mapping
// ============================================================================

/// Map a Gemini `FinishReason` string to our [`StopReason`].
///
/// Mirrors pi-mono's `mapStopReason` (the typed `FinishReason` variant).
pub fn map_stop_reason(reason: &str) -> StopReason {
    match reason {
        "STOP" => StopReason::Stop,
        "MAX_TOKENS" => StopReason::Length,
        // All other reasons are treated as errors.
        _ => StopReason::Error,
    }
}

/// Map a raw string finish reason to our [`StopReason`] (for raw API responses).
///
/// Mirrors pi-mono's `mapStopReasonString`.
pub fn map_stop_reason_string(reason: &str) -> StopReason {
    match reason {
        "STOP" => StopReason::Stop,
        "MAX_TOKENS" => StopReason::Length,
        _ => StopReason::Error,
    }
}

// ============================================================================
// Thought-signature validation
// ============================================================================

/// Sentinel value that tells the Gemini API to skip thought-signature
/// validation.  Used for unsigned function-call parts replayed from providers
/// that do not produce thought signatures (e.g. Claude via Antigravity).
///
/// See: <https://ai.google.dev/gemini-api/docs/thought-signatures>
pub const SKIP_THOUGHT_SIGNATURE: &str = "skip_thought_signature_validator";

/// Base64 pattern: A-Za-z0-9+/ with up to two trailing `=` padding characters.
fn is_valid_base64_thought_signature(signature: &str) -> bool {
    if signature.is_empty() {
        return false;
    }
    if signature.len() % 4 != 0 {
        return false;
    }
    signature
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
}

/// Determine the effective thought signature to send to a Google API for a
/// given part.
///
/// Rules (mirrors pi-mono's `resolveThoughtSignature` + Gemini 3 logic):
/// - Keep the signature only when replaying a response from the **same provider
///   and model** that produced it, and only when it passes base64 validation.
/// - For Gemini 3 models, unsigned function-call parts must use the
///   `SKIP_THOUGHT_SIGNATURE` sentinel instead of omitting the field entirely.
///
/// # Arguments
/// * `is_same_provider_and_model` — whether the source message came from the
///   exact same provider+model combination.
/// * `model_id` — the current model's ID.
/// * `signature` — the raw signature from the stored message block, if any.
pub fn resolve_effective_thought_signature(
    is_same_provider_and_model: bool,
    model_id: &str,
    signature: Option<&str>,
) -> Option<String> {
    let resolved = if is_same_provider_and_model {
        signature.filter(|s| is_valid_base64_thought_signature(s))
    } else {
        None
    };

    let is_gemini3_model = super::google::is_gemini3(model_id);

    if resolved.is_some() {
        resolved.map(String::from)
    } else if is_gemini3_model {
        // Gemini 3 requires the sentinel for unsigned function calls.
        Some(SKIP_THOUGHT_SIGNATURE.to_string())
    } else {
        None
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_thinking_part ─────────────────────────────────────────────────────

    #[test]
    fn test_is_thinking_part_thought_true() {
        assert!(is_thinking_part(Some(true), None));
        assert!(is_thinking_part(Some(true), Some("some_sig")));
    }

    #[test]
    fn test_is_thinking_part_thought_false() {
        assert!(!is_thinking_part(Some(false), None));
    }

    #[test]
    fn test_is_thinking_part_no_thought_field() {
        assert!(!is_thinking_part(None, None));
        // thoughtSignature alone does NOT make it a thinking part.
        assert!(!is_thinking_part(None, Some("AAAA")));
    }

    // ── retain_thought_signature ─────────────────────────────────────────────

    #[test]
    fn test_retain_thought_signature_incoming_non_empty() {
        assert_eq!(
            retain_thought_signature(Some("old"), Some("new")),
            Some("new")
        );
    }

    #[test]
    fn test_retain_thought_signature_incoming_empty() {
        assert_eq!(retain_thought_signature(Some("old"), Some("")), Some("old"));
    }

    #[test]
    fn test_retain_thought_signature_incoming_none() {
        assert_eq!(retain_thought_signature(Some("old"), None), Some("old"));
    }

    #[test]
    fn test_retain_thought_signature_both_none() {
        assert_eq!(retain_thought_signature(None, None), None);
    }

    #[test]
    fn test_retain_thought_signature_existing_none_incoming_set() {
        assert_eq!(retain_thought_signature(None, Some("new")), Some("new"));
    }

    // ── retain_thought_signature_owned ───────────────────────────────────────

    #[test]
    fn test_retain_owned_new_wins() {
        let result = retain_thought_signature_owned(Some("old".into()), Some("new".into()));
        assert_eq!(result.as_deref(), Some("new"));
    }

    #[test]
    fn test_retain_owned_empty_new_keeps_old() {
        let result = retain_thought_signature_owned(Some("old".into()), Some(String::new()));
        assert_eq!(result.as_deref(), Some("old"));
    }

    // ── requires_tool_call_id ────────────────────────────────────────────────

    #[test]
    fn test_requires_tool_call_id_claude() {
        assert!(requires_tool_call_id("claude-3-5-sonnet-20241022"));
        assert!(requires_tool_call_id("claude-opus-4-7"));
    }

    #[test]
    fn test_requires_tool_call_id_gpt_oss() {
        assert!(requires_tool_call_id("gpt-oss-something"));
    }

    #[test]
    fn test_requires_tool_call_id_gemini() {
        assert!(!requires_tool_call_id("gemini-2.5-pro"));
        assert!(!requires_tool_call_id("gemini-3-flash"));
    }

    // ── map_stop_reason ───────────────────────────────────────────────────────

    #[test]
    fn test_map_stop_reason_stop() {
        assert_eq!(map_stop_reason("STOP"), StopReason::Stop);
    }

    #[test]
    fn test_map_stop_reason_max_tokens() {
        assert_eq!(map_stop_reason("MAX_TOKENS"), StopReason::Length);
    }

    #[test]
    fn test_map_stop_reason_safety() {
        assert_eq!(map_stop_reason("SAFETY"), StopReason::Error);
    }

    #[test]
    fn test_map_stop_reason_unknown() {
        assert_eq!(map_stop_reason("SOMETHING_NEW"), StopReason::Error);
    }

    // ── map_stop_reason_string ────────────────────────────────────────────────

    #[test]
    fn test_map_stop_reason_string_stop() {
        assert_eq!(map_stop_reason_string("STOP"), StopReason::Stop);
    }

    #[test]
    fn test_map_stop_reason_string_max_tokens() {
        assert_eq!(map_stop_reason_string("MAX_TOKENS"), StopReason::Length);
    }

    #[test]
    fn test_map_stop_reason_string_other() {
        assert_eq!(map_stop_reason_string("BLOCKLIST"), StopReason::Error);
    }

    // ── is_valid_base64_thought_signature ─────────────────────────────────────

    #[test]
    fn test_valid_base64() {
        // "AAAA" is valid base64, length % 4 == 0.
        assert!(is_valid_base64_thought_signature("AAAA"));
    }

    #[test]
    fn test_invalid_base64_odd_length() {
        assert!(!is_valid_base64_thought_signature("AAA"));
    }

    #[test]
    fn test_invalid_base64_bad_chars() {
        assert!(!is_valid_base64_thought_signature("AA@!"));
    }

    #[test]
    fn test_invalid_base64_empty() {
        assert!(!is_valid_base64_thought_signature(""));
    }

    // ── resolve_effective_thought_signature ───────────────────────────────────

    #[test]
    fn test_resolve_same_provider_valid_sig() {
        // Valid base64 (length 4, chars OK), same provider → keep it.
        let result = resolve_effective_thought_signature(true, "gemini-2.5-pro", Some("AAAA"));
        assert_eq!(result.as_deref(), Some("AAAA"));
    }

    #[test]
    fn test_resolve_different_provider_drops_sig() {
        // Different provider → signature dropped.
        // Non-Gemini-3 model → no sentinel → None.
        let result = resolve_effective_thought_signature(false, "gemini-2.5-pro", Some("AAAA"));
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_gemini3_no_sig_gets_sentinel() {
        // Gemini 3, no valid sig → sentinel.
        let result = resolve_effective_thought_signature(false, "gemini-3-flash", None);
        assert_eq!(result.as_deref(), Some(SKIP_THOUGHT_SIGNATURE));
    }

    #[test]
    fn test_resolve_gemini3_same_provider_valid_sig_kept() {
        let sig = "AAAA"; // valid base64
        let result = resolve_effective_thought_signature(true, "gemini-3-flash", Some(sig));
        assert_eq!(result.as_deref(), Some(sig));
    }
}
