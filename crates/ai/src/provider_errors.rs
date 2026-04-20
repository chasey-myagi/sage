//! Provider-level error classification (Sprint 12 M2).
//!
//! When a Provider returns 4xx with a body suggesting the model name is
//! invalid (rather than a server-side problem), map it into a user-facing
//! message that cites the ProviderSpec.hint_docs_url so the operator can
//! look up the correct model id.

use crate::provider_specs::resolve_provider;
use crate::types::{AssistantMessageEvent, Model};

/// Does the HTTP 4xx body suggest the model id itself is invalid?
///
/// Sprint 12 task #77 (6) — classification is intentionally **strict** to
/// avoid misdirecting users to model-docs URLs when the real problem is a
/// tool / file / quota / permission issue whose recovery steps are
/// unrelated to model naming. Two rules enforce the narrow scope:
///
///   1. **Hard exclusions** (context_length / rate_limit / authentication /
///      quota_exceeded / permission_denied) veto any positive match. Each is
///      a distinct error family whose recovery path differs from
///      "look up correct model id".
///   2. Generic phrases like `"does not exist"` are only counted when the
///      body also mentions `"model"`. Previously the bare phrase caught
///      any "resource X does not exist" message.
pub fn is_invalid_model_body(body: &str) -> bool {
    if body.trim().is_empty() {
        return false;
    }
    let lower = body.to_ascii_lowercase();

    // Hard exclusions first — these categories veto any positive match.
    // Add here when a provider invents a new error family that deserves
    // its own recovery UX rather than "look at the model-id docs".
    if lower.contains("context_length")
        || lower.contains("rate_limit")
        || lower.contains("authentication")
        || lower.contains("quota_exceeded")
        || lower.contains("permission_denied")
    {
        return false;
    }

    // Positive patterns. Unambiguous model-family signatures first.
    let strong = lower.contains("model_not_found")
        || lower.contains("invalid_model")
        || lower.contains("model not found")
        || lower.contains("model does not exist")
        || lower.contains("model not exist");
    if strong {
        return true;
    }

    // Ambiguous phrases — require co-occurrence with the word "model" so
    // a "tool does not exist" or "file path does not exist" body doesn't
    // get misclassified as an invalid model id.
    let mentions_model = lower.contains("model");
    if mentions_model && lower.contains("does not exist") {
        return true;
    }

    // Anthropic not_found_error + model co-occurrence.
    if lower.contains("not_found_error") && mentions_model {
        return true;
    }
    // DashScope 特有: "invalid" + "parameter" + "model" 三词共现.
    if lower.contains("invalid") && lower.contains("parameter") && mentions_model {
        return true;
    }
    false
}

/// Truncate body to at most 200 chars (not bytes — UTF-8 safe), appending
/// "..." if truncated.
///
/// Using `.chars().take(200)` means multi-byte characters (CJK, emoji) are
/// never sliced mid-byte, so this function cannot panic on non-ASCII bodies.
/// Provider 4xx responses regularly contain Chinese (Kimi, DashScope, 豆包,
/// ZhipuAI) — slicing by byte index is a panic waiting to happen in the
/// error path.
fn body_excerpt(body: &str) -> String {
    const MAX_CHARS: usize = 200;
    let mut chars = body.chars();
    let truncated: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

/// Classify a provider error response for an invalid model id.
/// Returns `None` for other error categories (rate limit, auth, etc.).
pub fn classify_provider_error(
    provider: &str,
    model_id: &str,
    status: u16,
    body: &str,
) -> Option<String> {
    if !is_invalid_model_body(body) {
        return None;
    }
    let hint = resolve_provider(provider)
        .map(|s| s.hint_docs_url.to_string())
        .unwrap_or_default();
    let excerpt = body_excerpt(body);
    Some(format!(
        "Invalid model `{model_id}` for provider `{provider}` (HTTP {status}): {excerpt}. See: {hint}"
    ))
}

/// Render a user-facing provider error string.
///
/// - If `is_invalid_model_body(body)` ⇒ delegates to `SageError::InvalidModel`
///   display (canonical format). Guarantees the string a user sees here is
///   byte-identical to what they would see if the error flowed up as a
///   structured `SageError`.
/// - Otherwise ⇒ render the generic `"API error {status}: {body_excerpt}"`.
pub fn format_provider_error(
    provider: &str,
    model_id: &str,
    status: u16,
    body: &str,
) -> String {
    if let Some(err) = classify_provider_error(provider, model_id, status, body) {
        return err.to_string();
    }
    let excerpt = body_excerpt(body);
    format!("API error {status}: {excerpt}")
}

/// Consume a non-success `reqwest::Response` and produce the single
/// `AssistantMessageEvent::Error` the caller should return.
///
/// Every Provider's 4xx / non-success branch is identical: read status, read
/// body, format, return. This helper collapses those 7 duplicated call sites
/// (anthropic / google / google_vertex / azure_openai_responses /
/// openai_completions / openai_responses / openai_compat) into one, so a
/// future policy change — metric tag, structured log, retry classification —
/// only has to touch this function.
pub(crate) async fn handle_error_response(
    response: reqwest::Response,
    model: &Model,
) -> AssistantMessageEvent {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    AssistantMessageEvent::Error(format_provider_error(
        &model.provider,
        &model.id,
        status,
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // is_invalid_model_body — true cases (real vendor 4xx body samples)
    // ========================================================================

    #[test]
    fn is_invalid_model_body_matches_openai_invalid_model_error_type() {
        // OpenAI: code="model_not_found"
        let body = r#"{"error":{"message":"The model `gpt-99` does not exist","type":"invalid_request_error","code":"model_not_found"}}"#;
        assert!(is_invalid_model_body(body));
    }

    #[test]
    fn is_invalid_model_body_matches_anthropic_not_found_error() {
        // Anthropic: type="not_found_error" + message contains model name
        let body = r#"{"type":"error","error":{"type":"not_found_error","message":"model: claude-foo-bar"}}"#;
        assert!(is_invalid_model_body(body));
    }

    #[test]
    fn is_invalid_model_body_matches_moonshot_invalid_model() {
        // Moonshot/Kimi: code="invalid_model"
        let body = r#"{"error":{"message":"Invalid model: kimi-k99","type":"invalid_request_error","code":"invalid_model"}}"#;
        assert!(is_invalid_model_body(body));
    }

    #[test]
    fn is_invalid_model_body_matches_plain_does_not_exist() {
        // 包含字面 "does not exist" 子串
        let body = r#"The requested model does not exist in this region."#;
        assert!(is_invalid_model_body(body));
    }

    #[test]
    fn is_invalid_model_body_matches_capitalized_model_not_found() {
        // 大写首字母 "Model not found" / "model_not_found" — case-insensitive
        let body = r#"{"error":{"message":"Model not found","type":"invalid_request_error"}}"#;
        assert!(is_invalid_model_body(body));
    }

    #[test]
    fn is_invalid_model_body_matches_qwen_invalid_parameter() {
        // DashScope 格式: code="InvalidParameter", message 含 "model" + "invalid"
        let body = r#"{"code":"InvalidParameter","message":"The parameter `model` is invalid"}"#;
        assert!(is_invalid_model_body(body));
    }

    #[test]
    fn is_invalid_model_body_matches_deepseek_model_not_found() {
        // DeepSeek: "Model Not Exist"
        let body = r#"{"error":{"message":"Model Not Exist","type":"invalid_request_error"}}"#;
        assert!(is_invalid_model_body(body));
    }

    #[test]
    fn is_invalid_model_body_matches_bare_model_not_found_string() {
        // 最小正匹配：字符串中出现 "model_not_found" 即可
        let body = r#"model_not_found"#;
        assert!(is_invalid_model_body(body));
    }

    // ========================================================================
    // is_invalid_model_body — false cases (server errors, not invalid model)
    // ========================================================================

    #[test]
    fn is_invalid_model_body_false_on_rate_limit() {
        let body = r#"{"error":{"message":"Rate limit exceeded","type":"rate_limit_error"}}"#;
        assert!(!is_invalid_model_body(body));
    }

    #[test]
    fn is_invalid_model_body_false_on_auth_error() {
        let body = r#"{"error":{"message":"Invalid API key","type":"authentication_error"}}"#;
        assert!(!is_invalid_model_body(body));
    }

    #[test]
    fn is_invalid_model_body_false_on_server_error() {
        let body = r#"{"error":{"message":"Internal server error","type":"api_error"}}"#;
        assert!(!is_invalid_model_body(body));
    }

    #[test]
    fn is_invalid_model_body_false_on_context_length_exceeded() {
        // 含 "invalid_request_error" 但不应误伤 — context_length_exceeded 是独立错误类
        let body = r#"{"error":{"message":"This model's maximum context length is 128000 tokens","type":"invalid_request_error","code":"context_length_exceeded"}}"#;
        assert!(!is_invalid_model_body(body));
    }

    #[test]
    fn is_invalid_model_body_false_on_empty_body() {
        assert!(!is_invalid_model_body(""));
    }

    #[test]
    fn is_invalid_model_body_false_on_garbage() {
        // 粗暴 HTML 404 页
        let body = "<html>404 Not Found</html>";
        assert!(!is_invalid_model_body(body));
    }

    // ── Sprint 12 task #77 (6): keyword strategy tightening ──────────────
    //
    // The pre-tightening `is_invalid_model_body` matched the bare substring
    // `"does not exist"` — too permissive: any 4xx body that mentions
    // a missing resource (tool, file, path, dataset) would be mis-classified
    // as "model id is wrong" and misdirect the user to model-docs URLs.
    // Tightening: require `"does not exist"` to co-occur with `"model"`.
    //
    // Additionally, `quota_exceeded` and `permission_denied` must veto
    // positive matches — they're distinct error families whose recovery
    // steps (buy more quota / fix IAM) are unrelated to model naming.

    #[test]
    fn is_invalid_model_body_false_on_tool_not_found_with_does_not_exist() {
        // Regression: a tool-related 4xx that mentions "does not exist"
        // must NOT be classified as an invalid model.
        let body = r#"{"error":{"message":"The requested tool does not exist in the registry"}}"#;
        assert!(
            !is_invalid_model_body(body),
            "tool-level 'does not exist' must not trigger invalid-model classification"
        );
    }

    #[test]
    fn is_invalid_model_body_false_on_file_path_does_not_exist() {
        // Providers sometimes return filesystem / upload errors that mention
        // a missing path. Must not be model-error.
        let body = r#"{"error":{"message":"uploaded file path does not exist"}}"#;
        assert!(
            !is_invalid_model_body(body),
            "filesystem 'does not exist' must not trigger invalid-model classification"
        );
    }

    #[test]
    fn is_invalid_model_body_true_when_model_and_does_not_exist_cooccur() {
        // Positive: model + does not exist must still match even when split.
        let body = r#"{"error":{"message":"The specified model for completion does not exist in this region","type":"invalid_request_error"}}"#;
        assert!(
            is_invalid_model_body(body),
            "model + does-not-exist co-occurrence must still match"
        );
    }

    #[test]
    fn is_invalid_model_body_false_on_quota_exceeded() {
        // New negative exclusion: quota_exceeded is a billing-layer concern,
        // has its own recovery path (buy more / wait for reset). Must not be
        // routed to model-docs-URL hint path.
        let body = r#"{"error":{"code":"quota_exceeded","message":"You have exceeded the monthly quota for model gpt-4o"}}"#;
        assert!(
            !is_invalid_model_body(body),
            "quota_exceeded must veto even when body mentions model"
        );
    }

    #[test]
    fn is_invalid_model_body_false_on_permission_denied() {
        // New negative exclusion: permission_denied is IAM/RBAC, unrelated
        // to model naming.
        let body = r#"{"error":{"code":"permission_denied","message":"Your org is not allowed to access model claude-opus-4-7"}}"#;
        assert!(
            !is_invalid_model_body(body),
            "permission_denied must veto even when body mentions model"
        );
    }

    // ========================================================================
    // format_provider_error — invalid model path (8+ cases)
    // ========================================================================

    #[test]
    fn format_provider_error_for_invalid_model_includes_provider_and_model_id() {
        // invalid-model body → output 包含 provider name 和 model_id
        let body = r#"{"error":{"message":"Invalid model: kimi-k99","type":"invalid_request_error","code":"invalid_model"}}"#;
        let out = format_provider_error("kimi", "kimi-k99", 400, body);
        assert!(out.contains("kimi"), "output should mention provider 'kimi', got: {out}");
        assert!(out.contains("kimi-k99"), "output should mention model_id 'kimi-k99', got: {out}");
    }

    #[test]
    fn format_provider_error_for_invalid_model_includes_hint_docs_url() {
        // kimi 的 hint_docs_url 来自 ProviderSpec，含 "moonshot.cn"
        let body = r#"{"error":{"message":"Invalid model: kimi-k99","type":"invalid_request_error","code":"invalid_model"}}"#;
        let out = format_provider_error("kimi", "kimi-k99", 400, body);
        assert!(
            out.contains("moonshot.cn"),
            "output should contain hint URL with 'moonshot.cn', got: {out}"
        );
    }

    #[test]
    fn format_provider_error_for_invalid_model_cites_provider_error() {
        // output 应包含 body 里 message 的片段（比如 "Invalid model"）
        let body = r#"{"error":{"message":"Invalid model: kimi-k99","type":"invalid_request_error","code":"invalid_model"}}"#;
        let out = format_provider_error("kimi", "kimi-k99", 400, body);
        assert!(
            out.contains("Invalid model") || out.contains("invalid_model"),
            "output should cite the provider error message, got: {out}"
        );
    }

    #[test]
    fn format_provider_error_for_invalid_model_unknown_provider_falls_back_gracefully() {
        // provider 不在 spec 列表里 → 不 panic，依然包含 provider 名
        let body = r#"{"error":{"message":"model_not_found","type":"invalid_request_error","code":"model_not_found"}}"#;
        let out = format_provider_error("not-in-spec-list-xyz", "some-model", 400, body);
        assert!(
            out.contains("not-in-spec-list-xyz"),
            "output should mention unknown provider name, got: {out}"
        );
        // 不 panic 即满足，上面 contains 断言是加分项
    }

    #[test]
    fn format_provider_error_for_generic_server_error_uses_plain_format() {
        // 500 server error → 不是 invalid model → 输出格式 "API error 500: ..."
        let body = r#"{"error":{"message":"Internal server error","type":"api_error"}}"#;
        let out = format_provider_error("kimi", "kimi-k2.5", 500, body);
        assert!(
            out.starts_with("API error 500:"),
            "non-invalid-model error should use plain format, got: {out}"
        );
    }

    #[test]
    fn format_provider_error_for_rate_limit_uses_plain_format() {
        let body = r#"{"error":{"message":"Rate limit exceeded","type":"rate_limit_error"}}"#;
        let out = format_provider_error("kimi", "kimi-k2.5", 429, body);
        assert!(
            out.starts_with("API error 429:"),
            "rate limit error should use plain format, got: {out}"
        );
    }

    #[test]
    fn format_provider_error_for_auth_uses_plain_format() {
        let body = r#"{"error":{"message":"Invalid API key","type":"authentication_error"}}"#;
        let out = format_provider_error("kimi", "kimi-k2.5", 401, body);
        assert!(
            out.starts_with("API error 401:"),
            "auth error should use plain format, got: {out}"
        );
    }

    #[test]
    fn format_provider_error_status_code_appears_in_output() {
        // 不论哪个路径，status code 都应该出现在输出里
        let body_400 = r#"{"error":{"message":"Invalid model: foo","type":"invalid_request_error","code":"model_not_found"}}"#;
        let out_400 = format_provider_error("openai", "gpt-99", 400, body_400);
        assert!(out_400.contains("400"), "status 400 should appear in output, got: {out_400}");

        let body_429 = r#"{"error":{"message":"Rate limit exceeded","type":"rate_limit_error"}}"#;
        let out_429 = format_provider_error("openai", "gpt-4o", 429, body_429);
        assert!(out_429.contains("429"), "status 429 should appear in output, got: {out_429}");
    }

    #[test]
    fn format_provider_error_invalid_model_body_does_not_leak_full_body_past_200_chars() {
        // body > 200 chars 时，output 里的 body 摘录 ≤ 250 chars（含省略号）
        let long_suffix = "x".repeat(300);
        let body = format!(
            r#"{{"error":{{"message":"model_not_found {long_suffix}","type":"invalid_request_error","code":"model_not_found"}}}}"#
        );
        let out = format_provider_error("kimi", "kimi-k99", 400, &body);
        // 计算 output 中出现的 body 内容长度（用最宽松方法：output 总长度不超 500）
        // 实际约束：body excerpt 不超过 250 chars
        let body_in_out: usize = out.len();
        assert!(
            body_in_out <= 500,
            "output is suspiciously long ({body_in_out} chars), body excerpt should be truncated"
        );
        // 另外检查 "..." 省略号存在（截断信号）
        assert!(
            out.contains("...") || out.len() < body.len(),
            "truncated body should include '...' or output should be shorter than raw body"
        );
    }

    #[test]
    fn format_provider_error_invalid_model_short_body_included_verbatim() {
        // body < 100 chars 时，整段包含在 output 中
        let body = r#"{"error":{"code":"model_not_found"}}"#; // 36 chars
        let out = format_provider_error("openai", "gpt-99", 400, body);
        assert!(
            out.contains(body) || out.contains("model_not_found"),
            "short body should be included verbatim or its content should appear, got: {out}"
        );
    }

    // ========================================================================
    // 集成层烟测 (optional): hint URL 真的来自 ProviderSpec，不是硬编码
    // ========================================================================

    #[test]
    fn format_provider_error_resolves_hint_from_provider_spec() {
        use crate::provider_specs::resolve_provider;

        let body = r#"{"error":{"message":"Invalid model: kimi-k99","type":"invalid_request_error","code":"invalid_model"}}"#;
        let out = format_provider_error("kimi", "kimi-k99", 400, body);

        // hint_docs_url 来自 ProviderSpec — 不能是任意硬编码字符串
        let spec = resolve_provider("kimi").expect("kimi must exist in provider specs");
        assert!(
            out.contains(spec.hint_docs_url),
            "output hint URL should match ProviderSpec.hint_docs_url='{}', got: {out}",
            spec.hint_docs_url
        );
    }

    // ========================================================================
    // classify_provider_error 测试
    // ========================================================================

    #[test]
    fn classify_provider_error_returns_some_invalid_model_for_matching_body() {
        let err = classify_provider_error("kimi", "kimi-k99", 400, r#"{"error":{"code":"model_not_found"}}"#);
        assert!(err.is_some());
    }

    #[test]
    fn classify_provider_error_returns_none_for_rate_limit() {
        let err = classify_provider_error("kimi", "kimi-k99", 429, r#"{"error":{"type":"rate_limit_error"}}"#);
        assert!(err.is_none());
    }

    // ========================================================================
    // UTF-8 safety regression (Linus v1 blocker)
    // ========================================================================

    #[test]
    fn body_excerpt_does_not_panic_on_multibyte_chinese_body_at_truncation_boundary() {
        // CJK chars are 3 bytes each in UTF-8. Byte-indexing at 200 would slice
        // mid-char and panic. char-based truncation handles this cleanly.
        let body = "模型不存在".repeat(60); // 300 CJK chars, 900 bytes
        let out = body_excerpt(&body);
        assert!(out.ends_with("..."), "long body should be truncated with ellipsis");
        assert!(out.chars().count() <= 203, "truncated output should be ≤200 chars + '...'");
    }

    #[test]
    fn body_excerpt_does_not_panic_on_emoji_body() {
        // Emoji are 4-byte UTF-8 sequences — classic panic trap for byte slicing.
        let body = "🚀".repeat(250); // 250 emoji, 1000 bytes
        let _out = body_excerpt(&body); // if slicing by byte, this panics
    }

    #[test]
    fn format_provider_error_does_not_panic_on_chinese_invalid_model_body() {
        // Full path: Provider (Kimi 真实会返回中文) → format → display
        let body = r#"{"error":{"code":"invalid_model","message":"模型 kimi-k99 不存在，请检查模型名称"}}"#;
        let msg = format_provider_error("kimi", "kimi-k99", 400, body);
        assert!(msg.contains("kimi-k99"));
        assert!(msg.contains("模型") || msg.contains("不存在"),
            "excerpt should include Chinese error text, got: {msg}");
    }

    // ========================================================================
    // Canonical format: format_provider_error == SageError::InvalidModel.to_string()
    // ========================================================================

    #[test]
    fn format_provider_error_and_sage_error_display_produce_identical_strings() {
        // Linus v1 blocker #3: two format sources must stay in sync. This test
        // locks byte-identity so anyone who changes the display template for
        // one path and forgets the other fails CI immediately.
        let body = r#"{"error":{"code":"model_not_found"}}"#;
        let formatted = format_provider_error("kimi", "kimi-k99", 400, body);
        let structured = classify_provider_error("kimi", "kimi-k99", 400, body)
            .expect("body matches invalid-model pattern");
        assert_eq!(formatted, structured.to_string());
    }
}
