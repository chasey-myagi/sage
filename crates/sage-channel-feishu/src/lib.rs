//! Feishu (Lark) channel adapter for Sage.
//!
//! Sprint 8 scope:
//!   * `FeishuChannel` — outbound [`ChannelAdapter`] that delivers
//!     user-visible [`AgentEvent`]s to a Feishu chat via the Open Platform
//!     messaging API.
//!   * `parse_webhook_payload` — decode an inbound Feishu webhook body
//!     (`im.message.receive_v1` event) into a [`FeishuInboundMessage`].
//!   * `verify_signature` — HMAC-SHA256 signature check for inbound events.
//!   * `webhook_router` — an `axum::Router` that wires the above into an
//!     HTTP endpoint (`POST /webhook`), ready for the daemon to mount.
//!
//! The HTTP client used for outbound calls is abstracted behind the
//! [`HttpClient`] trait so unit tests can inject a fake and assert on the
//! request the channel builds, without making real network calls.
//!
//! Integration with `SageSession`'s hook bus (injecting `channel_hints`
//! into the system prompt at SessionStart) is explicitly out of scope for
//! this sprint; it is the job of the caller wiring layer.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use sage_runner::channel::ChannelAdapter;
use sage_runtime::event::{AgentEvent, Visibility};

// ─── HttpClient abstraction ────────────────────────────────────────────────

/// A minimal HTTP POST client surface, abstracted so tests can inject a fake.
///
/// The Feishu adapter only needs to POST JSON bodies with optional auth
/// headers — a full reqwest surface would be overkill (and untestable
/// without the network).
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// POST `body_json` (a serialised JSON string) to `url` with an optional
    /// `Authorization: Bearer <token>` header. Returns the response body as
    /// a string on success.
    async fn post_json(
        &self,
        url: &str,
        body_json: &str,
        bearer_token: Option<&str>,
    ) -> Result<String>;
}

/// Default implementation of [`HttpClient`] backed by `reqwest`.
pub struct ReqwestHttpClient {
    inner: reqwest::Client,
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        Self {
            inner: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn post_json(
        &self,
        url: &str,
        body_json: &str,
        bearer_token: Option<&str>,
    ) -> Result<String> {
        let mut req = self
            .inner
            .post(url)
            .header("Content-Type", "application/json")
            .body(body_json.to_string());
        if let Some(tok) = bearer_token {
            req = req.bearer_auth(tok);
        }
        let resp = req.send().await.context("feishu POST failed")?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("feishu API returned HTTP {status}: {text}"));
        }
        Ok(text)
    }
}

// ─── FeishuChannel ─────────────────────────────────────────────────────────

/// Feishu (Lark) channel adapter.
pub struct FeishuChannel {
    app_id: String,
    app_secret: String,
    verification_token: Option<String>,
    http: Arc<dyn HttpClient>,
    /// Default chat id used as the `receive_id` when outbound events don't
    /// carry one. In Sprint 8 this is set by the caller after an inbound
    /// message arrives; a `None` value means `send()` will drop the event
    /// (there's no one to reply to yet).
    default_chat_id: std::sync::Mutex<Option<String>>,
}

impl FeishuChannel {
    /// Construct a `FeishuChannel` using the default `reqwest` HTTP client.
    pub fn new(
        app_id: String,
        app_secret: String,
        verification_token: Option<String>,
    ) -> Self {
        Self::new_with_client(
            app_id,
            app_secret,
            verification_token,
            Arc::new(ReqwestHttpClient::default()),
        )
    }

    /// Construct a `FeishuChannel` with a custom [`HttpClient`] implementation.
    /// Used by tests to inject fake clients and inspect outbound requests.
    pub fn new_with_client(
        app_id: String,
        app_secret: String,
        verification_token: Option<String>,
        http: Arc<dyn HttpClient>,
    ) -> Self {
        Self {
            app_id,
            app_secret,
            verification_token,
            http,
            default_chat_id: std::sync::Mutex::new(None),
        }
    }

    /// Read accessors for tests / diagnostics.
    pub fn app_id(&self) -> &str {
        &self.app_id
    }
    pub fn app_secret(&self) -> &str {
        &self.app_secret
    }
    pub fn verification_token(&self) -> Option<&str> {
        self.verification_token.as_deref()
    }

    /// Set the default chat id used when an event doesn't carry one.
    /// Typically called when an inbound message arrives via `webhook_router`.
    ///
    /// Transition-phase semantics: backed by a `Mutex`; concurrent writers
    /// race and the last write wins. Acceptable for Sprint 8 (single active
    /// chat). Multi-chat routing must be refactored at the Session side before
    /// production use.
    pub(crate) fn set_default_chat_id(&self, chat_id: String) {
        if let Ok(mut guard) = self.default_chat_id.lock() {
            *guard = Some(chat_id);
        }
    }

    fn current_chat_id(&self) -> Option<String> {
        self.default_chat_id.lock().ok().and_then(|g| g.clone())
    }

    /// Build the Feishu `im/v1/messages` JSON body for a plain text message.
    /// Exposed for unit tests so we can assert the wire shape without the
    /// network.
    pub fn build_text_message_body(chat_id: &str, text: &str) -> serde_json::Value {
        serde_json::json!({
            "receive_id": chat_id,
            "msg_type": "text",
            "content": serde_json::to_string(&serde_json::json!({ "text": text }))
                .expect("json encode"),
        })
    }

    /// Extract the text payload of an event that should be forwarded to the
    /// channel, or `None` if this event carries no user-visible text.
    ///
    /// Current policy: forward the streaming `delta` of `MessageUpdate` and
    /// the `error` string of `RunError` (both already classified as
    /// [`Visibility::User`]). Other variants fall through to `None`.
    fn extract_text(event: &AgentEvent) -> Option<String> {
        match event {
            AgentEvent::MessageUpdate { delta, .. } => {
                if delta.is_empty() {
                    None
                } else {
                    Some(delta.clone())
                }
            }
            AgentEvent::RunError { error } => Some(format!("⚠ {error}")),
            _ => None,
        }
    }
}

#[async_trait]
impl ChannelAdapter for FeishuChannel {
    fn name(&self) -> &str {
        "feishu"
    }

    fn channel_hints(&self) -> &str {
        // Concise instructions shipped into the system prompt.
        concat!(
            "Platform: Feishu (Lark).\n",
            "- Reply in plain text or Feishu-flavoured markdown.\n",
            "- To mention a user, use <at user_id=\"...\">@name</at>.\n",
            "- Keep individual messages under 4000 characters.\n",
            "- Code blocks are supported via triple backticks.\n",
        )
    }

    fn visibility_filter(&self) -> Visibility {
        Visibility::User
    }

    async fn send(&self, event: AgentEvent) -> Result<()> {
        // Drop events that are not intended for end users.
        if event.visibility() != self.visibility_filter() {
            return Ok(());
        }
        let Some(text) = Self::extract_text(&event) else {
            return Ok(());
        };
        let Some(chat_id) = self.current_chat_id() else {
            tracing::debug!(
                channel = "feishu",
                "no default chat_id set — dropping outbound event"
            );
            return Ok(());
        };

        let body = Self::build_text_message_body(&chat_id, &text);
        let body_json =
            serde_json::to_string(&body).context("serialize feishu message body")?;
        // TODO (sprint 8 impl): refresh tenant_access_token using app_id/app_secret.
        // For now the HttpClient receives no bearer — tests use a fake client.
        let _ = self
            .http
            .post_json(
                "https://open.feishu.cn/open-apis/im/v1/messages",
                &body_json,
                None,
            )
            .await?;
        Ok(())
    }
}

// ─── Inbound webhook parsing ──────────────────────────────────────────────

/// A decoded inbound Feishu message (stripped of platform envelope).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuInboundMessage {
    pub message_id: String,
    pub chat_id: String,
    pub sender_id: String,
    /// Pure text content with `<at ...>...</at>` mention tags removed.
    pub text: String,
}

/// Parse a raw Feishu webhook body (JSON) into a [`FeishuInboundMessage`].
///
/// Accepts the `im.message.receive_v1` event shape:
///
/// ```json
/// {
///   "schema": "2.0",
///   "header": { "event_type": "im.message.receive_v1", ... },
///   "event": {
///     "sender": { "sender_id": { "open_id": "ou_xxx" }, ... },
///     "message": {
///       "message_id": "om_xxx",
///       "chat_id": "oc_xxx",
///       "message_type": "text",
///       "content": "{\"text\":\"<at user_id=\\\"xxx\\\">@bot</at> hello\"}"
///     }
///   }
/// }
/// ```
///
/// Fails on empty body, malformed JSON, or non-message event types.
pub fn parse_webhook_payload(body: &[u8]) -> Result<FeishuInboundMessage> {
    if body.is_empty() {
        return Err(anyhow!("empty webhook body"));
    }
    let root: serde_json::Value =
        serde_json::from_slice(body).context("invalid webhook JSON")?;

    // Accept either the v2 schema (`header.event_type`) or the older v1 form
    // (`type: "event_callback"`).
    let event_type = root
        .get("header")
        .and_then(|h| h.get("event_type"))
        .and_then(|v| v.as_str())
        .or_else(|| root.get("type").and_then(|v| v.as_str()))
        .ok_or_else(|| anyhow!("missing event_type in webhook payload"))?;
    if event_type != "im.message.receive_v1" {
        return Err(anyhow!(
            "unsupported event_type: {event_type} (only im.message.receive_v1 is handled)"
        ));
    }

    let event = root
        .get("event")
        .ok_or_else(|| anyhow!("missing `event` object"))?;
    let message = event
        .get("message")
        .ok_or_else(|| anyhow!("missing `event.message` object"))?;

    let message_id = message
        .get("message_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("missing message.message_id"))?
        .to_string();
    let chat_id = message
        .get("chat_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("missing message.chat_id"))?
        .to_string();

    let sender_id = event
        .get("sender")
        .and_then(|s| s.get("sender_id"))
        .and_then(|s| s.get("open_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("missing sender.sender_id.open_id"))?
        .to_string();

    let content_str = message
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("missing message.content string"))?;
    let content_json: serde_json::Value =
        serde_json::from_str(content_str).context("message.content is not valid JSON")?;
    let raw_text = content_json
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(FeishuInboundMessage {
        message_id,
        chat_id,
        sender_id,
        text: strip_at_tags(&raw_text),
    })
}

/// Remove `<at user_id="...">@name</at>` tags from a Feishu message body,
/// keeping the surrounding text. Leading/trailing whitespace is trimmed.
///
/// Strategy (chosen for Sprint 8): drop the entire `<at>...</at>` block —
/// including the visible `@name` inside it — because mentions of the bot
/// are almost always noise for the agent's own processing. Alternate
/// strategies (keep `@name`, keep user_id) can be introduced later if
/// specific agents need them.
fn strip_at_tags(s: &str) -> String {
    // Hand-rolled scan rather than pulling in `regex` as a new dep.
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("<at") {
        out.push_str(&rest[..start]);
        // Find the closing </at>. If missing, emit the rest verbatim.
        if let Some(end_rel) = rest[start..].find("</at>") {
            let after = start + end_rel + "</at>".len();
            rest = &rest[after..];
        } else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

// ─── Signature verification ───────────────────────────────────────────────

/// Verify an inbound webhook signature.
///
/// Feishu's scheme is `HMAC-SHA256(key = verification_token, msg = timestamp + body)`,
/// hex-encoded. An empty token is treated as verification disabled → returns
/// `false` unconditionally (callers must not invoke this in that state; the
/// `webhook_router` skips signature checks when the channel has no token).
pub fn verify_signature(
    token: &str,
    timestamp: &str,
    body: &[u8],
    received_signature: &str,
) -> bool {
    if token.is_empty() || received_signature.is_empty() {
        return false;
    }
    type HmacSha256 = Hmac<Sha256>;
    let Ok(mut mac) = HmacSha256::new_from_slice(token.as_bytes()) else {
        return false;
    };
    mac.update(timestamp.as_bytes());
    mac.update(body);
    let computed = hex::encode(mac.finalize().into_bytes());
    // Constant-time compare.
    if computed.len() != received_signature.len() {
        return false;
    }
    let a = computed.as_bytes();
    let b = received_signature.as_bytes();
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Compute a valid signature for the given `(token, timestamp, body)` triple.
/// Exposed so tests (and integration callers) can generate signatures without
/// re-implementing the scheme.
pub fn compute_signature(token: &str, timestamp: &str, body: &[u8]) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(token.as_bytes()).expect("hmac key");
    mac.update(timestamp.as_bytes());
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

// ─── axum webhook router ──────────────────────────────────────────────────

/// A challenge-response payload used by Feishu during webhook URL
/// verification. The server must echo `challenge` back.
#[derive(Debug, Deserialize, Serialize)]
struct UrlVerification {
    challenge: String,
    #[serde(default)]
    token: Option<String>,
    #[serde(rename = "type", default)]
    ty: Option<String>,
}

/// Core handler logic for `POST /webhook/feishu`, extracted so unit tests can
/// call it directly without spinning up an HTTP server.
///
/// Steps:
///   1. Challenge echo (Feishu URL-verification flow).
///   2. Signature check when `channel` has a `verification_token`.
///   3. Parse the inbound event, record `chat_id`, return 200.
pub(crate) async fn handle_webhook(
    channel: Arc<FeishuChannel>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> (axum::http::StatusCode, String) {
    use axum::http::StatusCode;

    // URL verification challenge?
    if let Ok(uv) = serde_json::from_slice::<UrlVerification>(&body)
        && uv.ty.as_deref() == Some("url_verification")
    {
        return (
            StatusCode::OK,
            format!("{{\"challenge\":\"{}\"}}", uv.challenge),
        );
    }

    // Signature check (when token configured). No token = webhook accepts ALL
    // inbound traffic — log a warning per request to make the risk visible in
    // production logs, since operators could forget to set the token.
    if let Some(token) = channel.verification_token() {
        let ts = headers
            .get("X-Lark-Request-Timestamp")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let sig = headers
            .get("X-Lark-Signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !verify_signature(token, ts, &body, sig) {
            return (StatusCode::UNAUTHORIZED, "bad signature".to_string());
        }
    } else {
        tracing::warn!(
            channel = "feishu",
            "webhook accepted without signature verification — \
             set `verification_token` in channel config for production"
        );
    }

    match parse_webhook_payload(&body) {
        Ok(msg) => {
            channel.set_default_chat_id(msg.chat_id.clone());
            tracing::info!(
                channel = "feishu",
                chat_id = %msg.chat_id,
                message_id = %msg.message_id,
                "inbound feishu message"
            );
            (StatusCode::OK, "ok".to_string())
        }
        Err(err) => (axum::http::StatusCode::BAD_REQUEST, format!("parse error: {err}")),
    }
}

/// Build an `axum::Router` that accepts `POST /webhook` and dispatches
/// events to the given channel. Callers mount the returned router on their
/// own HTTP server (so the channel doesn't need to own the port binding).
pub fn webhook_router(channel: Arc<FeishuChannel>) -> axum::Router {
    use axum::Router;
    use axum::extract::State;
    use axum::routing::post;

    async fn handler(
        State(channel): State<Arc<FeishuChannel>>,
        headers: axum::http::HeaderMap,
        body: axum::body::Bytes,
    ) -> (axum::http::StatusCode, String) {
        handle_webhook(channel, headers, body).await
    }

    Router::new()
        .route("/webhook", post(handler))
        .with_state(channel)
}

// ═══════════════════════════════════════════════════════════════════════════
//                                 Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use sage_runtime::types::{AgentMessage, AssistantMessage};

    // ── FeishuChannel::new constructors ──────────────────────────────

    #[test]
    fn new_stores_fields_and_getters_return_them() {
        let ch = FeishuChannel::new(
            "cli_app".into(),
            "secret_xyz".into(),
            Some("vtok".into()),
        );
        assert_eq!(ch.app_id(), "cli_app");
        assert_eq!(ch.app_secret(), "secret_xyz");
        assert_eq!(ch.verification_token(), Some("vtok"));
        assert_eq!(ch.name(), "feishu");
        assert!(!ch.channel_hints().is_empty());
    }

    #[test]
    fn new_allows_none_verification_token() {
        let ch = FeishuChannel::new("cli_app".into(), "secret".into(), None);
        assert!(ch.verification_token().is_none());
    }

    // ── parse_webhook_payload ───────────────────────────────────────

    fn sample_receive_payload(raw_text: &str) -> Vec<u8> {
        let content = serde_json::json!({ "text": raw_text }).to_string();
        serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt_1",
                "event_type": "im.message.receive_v1",
                "create_time": "1710000000000",
                "token": "tok",
                "app_id": "cli_x",
                "tenant_key": "tk",
            },
            "event": {
                "sender": {
                    "sender_id": { "open_id": "ou_sender1", "union_id": "u1", "user_id": "us1" },
                    "sender_type": "user",
                    "tenant_key": "tk",
                },
                "message": {
                    "message_id": "om_msg1",
                    "root_id": "",
                    "parent_id": "",
                    "create_time": "1710000000000",
                    "chat_id": "oc_chat1",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": content,
                    "mentions": []
                }
            }
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn parse_typical_message_receive_payload() {
        let body = sample_receive_payload("hello world");
        let msg = parse_webhook_payload(&body).unwrap();
        assert_eq!(msg.message_id, "om_msg1");
        assert_eq!(msg.chat_id, "oc_chat1");
        assert_eq!(msg.sender_id, "ou_sender1");
        assert_eq!(msg.text, "hello world");
    }

    #[test]
    fn parse_strips_at_mention_tags() {
        // Strategy: <at ...>...</at> block is entirely removed, including
        // the visible @name. Surrounding text is kept and trimmed.
        let body = sample_receive_payload("<at user_id=\"ou_bot\">@bot</at> hello");
        let msg = parse_webhook_payload(&body).unwrap();
        assert_eq!(
            msg.text, "hello",
            "mention tag and its visible name must be stripped"
        );
    }

    #[test]
    fn parse_rejects_non_message_event_type() {
        let body = serde_json::json!({
            "schema": "2.0",
            "header": { "event_type": "contact.user.created_v3" },
            "event": {}
        })
        .to_string()
        .into_bytes();
        let err = parse_webhook_payload(&body).unwrap_err();
        assert!(
            err.to_string().contains("unsupported event_type"),
            "error should mention unsupported event_type, got: {err}"
        );
    }

    #[test]
    fn parse_rejects_malformed_json() {
        let body = b"{not valid json";
        let err = parse_webhook_payload(body).unwrap_err();
        assert!(err.to_string().contains("invalid webhook JSON"));
    }

    #[test]
    fn parse_rejects_empty_body() {
        let err = parse_webhook_payload(&[]).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    // ── verify_signature ────────────────────────────────────────────

    #[test]
    fn verify_signature_accepts_valid_signature() {
        let token = "my-verification-token";
        let ts = "1710000000";
        let body = b"{\"ping\":1}";
        let sig = compute_signature(token, ts, body);
        assert!(verify_signature(token, ts, body, &sig));
    }

    #[test]
    fn verify_signature_rejects_wrong_signature() {
        let token = "my-verification-token";
        let ts = "1710000000";
        let body = b"{\"ping\":1}";
        assert!(!verify_signature(token, ts, body, "deadbeef"));
    }

    #[test]
    fn verify_signature_empty_token_is_always_false() {
        // Callers with no token configured must not invoke verify_signature;
        // documented policy is that an empty token returns false
        // unconditionally.
        let sig = compute_signature("realtoken", "1710000000", b"abc");
        assert!(!verify_signature("", "1710000000", b"abc", &sig));
    }

    #[test]
    fn verify_signature_detects_wrong_timestamp() {
        let token = "my-verification-token";
        let body = b"payload";
        let sig = compute_signature(token, "1710000000", body);
        assert!(!verify_signature(token, "1710000099", body, &sig));
    }

    #[test]
    fn verify_signature_detects_body_tamper() {
        let token = "my-verification-token";
        let ts = "1710000000";
        let sig = compute_signature(token, ts, b"original");
        assert!(!verify_signature(token, ts, b"tampered", &sig));
    }

    // ── send(AgentEvent) via fake HttpClient ────────────────────────

    struct FakeClient {
        calls: Mutex<Vec<(String, String)>>, // (url, body)
    }

    impl FakeClient {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
            })
        }
        fn calls(&self) -> Vec<(String, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl HttpClient for FakeClient {
        async fn post_json(
            &self,
            url: &str,
            body_json: &str,
            _bearer_token: Option<&str>,
        ) -> Result<String> {
            self.calls
                .lock()
                .unwrap()
                .push((url.to_string(), body_json.to_string()));
            Ok("{\"code\":0}".into())
        }
    }

    fn channel_with_fake(fake: Arc<FakeClient>) -> FeishuChannel {
        FeishuChannel::new_with_client(
            "cli_x".into(),
            "sec".into(),
            None,
            fake as Arc<dyn HttpClient>,
        )
    }

    #[tokio::test]
    async fn send_text_delta_builds_feishu_text_payload() {
        let fake = FakeClient::new();
        let ch = channel_with_fake(fake.clone());
        ch.set_default_chat_id("oc_chat1".into());

        let ev = AgentEvent::MessageUpdate {
            message: AgentMessage::Assistant(AssistantMessage::new("hello".into())),
            delta: "hello".into(),
        };
        ch.send(ev).await.unwrap();

        let calls = fake.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].0,
            "https://open.feishu.cn/open-apis/im/v1/messages"
        );
        let body: serde_json::Value = serde_json::from_str(&calls[0].1).unwrap();
        assert_eq!(body["receive_id"], "oc_chat1");
        assert_eq!(body["msg_type"], "text");
        // content is a *string* (Feishu API requires JSON-as-string).
        let content_str = body["content"].as_str().unwrap();
        let content: serde_json::Value = serde_json::from_str(content_str).unwrap();
        assert_eq!(content["text"], "hello");
    }

    #[tokio::test]
    async fn send_developer_visibility_event_is_dropped() {
        let fake = FakeClient::new();
        let ch = channel_with_fake(fake.clone());
        ch.set_default_chat_id("oc_chat1".into());

        let ev = AgentEvent::ToolExecutionStart {
            tool_call_id: "tc-1".into(),
            tool_name: "bash".into(),
            args: serde_json::json!({"cmd": "ls"}),
        };
        ch.send(ev).await.unwrap();

        assert!(
            fake.calls().is_empty(),
            "developer-visibility event must not trigger an outbound HTTP call"
        );
    }

    #[tokio::test]
    async fn send_run_error_is_forwarded_as_text() {
        let fake = FakeClient::new();
        let ch = channel_with_fake(fake.clone());
        ch.set_default_chat_id("oc_chat1".into());

        let ev = AgentEvent::RunError {
            error: "boom".into(),
        };
        ch.send(ev).await.unwrap();

        let calls = fake.calls();
        assert_eq!(calls.len(), 1);
        let body: serde_json::Value = serde_json::from_str(&calls[0].1).unwrap();
        let content: serde_json::Value =
            serde_json::from_str(body["content"].as_str().unwrap()).unwrap();
        let text = content["text"].as_str().unwrap();
        assert!(
            text.contains("boom"),
            "RunError text should include the error message, got: {text}"
        );
    }

    #[tokio::test]
    async fn send_without_default_chat_id_drops_event() {
        // No set_default_chat_id() call — the channel hasn't seen an inbound
        // message yet. User-visibility events should be silently dropped
        // rather than 4xx'd or panicked.
        let fake = FakeClient::new();
        let ch = channel_with_fake(fake.clone());

        let ev = AgentEvent::MessageUpdate {
            message: AgentMessage::Assistant(AssistantMessage::new("hi".into())),
            delta: "hi".into(),
        };
        ch.send(ev).await.unwrap();

        assert!(fake.calls().is_empty());
    }

    // ── handle_webhook: handler-level unit tests ────────────────────

    fn make_channel_arc(token: Option<&str>) -> Arc<FeishuChannel> {
        Arc::new(FeishuChannel::new(
            "cli_x".into(),
            "sec".into(),
            token.map(String::from),
        ))
    }

    /// Build a minimal `HeaderMap` with the given key-value pairs.
    fn headers_with(pairs: &[(&str, &str)]) -> axum::http::HeaderMap {
        let mut map = axum::http::HeaderMap::new();
        for (k, v) in pairs {
            map.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                axum::http::HeaderValue::from_str(v).unwrap(),
            );
        }
        map
    }

    #[tokio::test]
    async fn webhook_handler_challenge_returns_200_with_echo() {
        let channel = make_channel_arc(None);
        let body = serde_json::json!({
            "challenge": "test-challenge-abc",
            "token": "tok",
            "type": "url_verification"
        })
        .to_string();

        let (status, resp_body) = handle_webhook(
            channel,
            headers_with(&[]),
            axum::body::Bytes::from(body),
        )
        .await;

        assert_eq!(status, axum::http::StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
        assert_eq!(v["challenge"], "test-challenge-abc");
    }

    #[tokio::test]
    async fn webhook_handler_bad_signature_returns_401() {
        let channel = make_channel_arc(Some("real-token"));
        let body = sample_receive_payload("hello");
        let body_bytes = axum::body::Bytes::from(body);

        // Provide a deliberately wrong signature.
        let headers = headers_with(&[
            ("X-Lark-Request-Timestamp", "1710000000"),
            ("X-Lark-Signature", "deadbeefdeadbeef"),
        ]);

        let (status, _) = handle_webhook(channel, headers, body_bytes).await;
        assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn webhook_handler_valid_message_returns_200_and_sets_chat_id() {
        let channel = make_channel_arc(None); // no token → skip sig check
        let body = sample_receive_payload("hi from feishu");
        let body_bytes = axum::body::Bytes::from(body.clone());

        let (status, _) = handle_webhook(
            channel.clone(),
            headers_with(&[]),
            body_bytes,
        )
        .await;

        assert_eq!(status, axum::http::StatusCode::OK);
        // Chat id from sample_receive_payload fixture is "oc_chat1".
        assert_eq!(
            channel.current_chat_id().as_deref(),
            Some("oc_chat1"),
            "handle_webhook must call set_default_chat_id with the parsed chat_id"
        );
    }

    #[tokio::test]
    async fn webhook_handler_valid_sig_bad_json_returns_400() {
        let channel = Arc::new(FeishuChannel::new(
            "test_app".to_string(),
            "test_secret".to_string(),
            Some("test_token".to_string()),
        ));

        let body = b"not valid json at all";
        let ts = "1710000000";
        let sig = compute_signature("test_token", ts, body);

        let headers = headers_with(&[
            ("X-Lark-Signature", &sig),
            ("X-Lark-Request-Timestamp", ts),
        ]);

        let (status, _body) = handle_webhook(
            channel,
            headers,
            axum::body::Bytes::from_static(b"not valid json at all"),
        )
        .await;

        assert_eq!(
            status,
            axum::http::StatusCode::BAD_REQUEST,
            "malformed JSON after valid sig must 400"
        );
    }
}
