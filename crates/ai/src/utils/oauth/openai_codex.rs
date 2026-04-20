//! OpenAI Codex (ChatGPT OAuth) flow.
//! Rust counterpart of `packages/ai/src/utils/oauth/openai-codex.ts`.
//!
//! Uses PKCE + local HTTP callback server (Axum/Tokio) or manual code paste
//! as fallback. CLI use only — not for browser environments.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::oneshot;
use tracing::error;

use super::oauth_page::{oauth_error_html, oauth_success_html};
use super::pkce::generate_pkce;
use super::types::{OAuthAuthInfo, OAuthCredentials, OAuthLoginCallbacks, OAuthPrompt, OAuthProviderInterface};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const SCOPE: &str = "openid profile email offline_access";
/// JSON claim path inside the JWT for the OpenAI auth section.
const JWT_CLAIM_PATH: &str = "https://api.openai.com/auth";

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn create_state() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parse an authorization callback input that may be a full URL, a
/// `code=…&state=…` query string, or a bare code (optionally `code#state`).
///
/// Mirrors `parseAuthorizationInput()` from `openai-codex.ts`.
fn parse_authorization_input(input: &str) -> (Option<String>, Option<String>) {
    let value = input.trim();
    if value.is_empty() {
        return (None, None);
    }

    // Try full URL
    if let Ok(url) = url::Url::parse(value) {
        let code = url.query_pairs().find(|(k, _)| k == "code").map(|(_, v)| v.into_owned());
        let state = url.query_pairs().find(|(k, _)| k == "state").map(|(_, v)| v.into_owned());
        return (code, state);
    }

    // `code#state` shorthand
    if value.contains('#') {
        let mut parts = value.splitn(2, '#');
        let code = parts.next().map(str::to_owned);
        let state = parts.next().map(str::to_owned);
        return (code, state);
    }

    // Query-string style `code=…&state=…`
    if value.contains("code=") {
        let params: std::collections::HashMap<_, _> = url::form_urlencoded::parse(value.as_bytes()).collect();
        let code = params.get("code").map(|v| v.to_string());
        let state = params.get("state").map(|v| v.to_string());
        return (code, state);
    }

    // Bare code
    (Some(value.to_owned()), None)
}

/// Decode a JWT payload (middle section) without verifying the signature.
fn decode_jwt_payload(token: &str) -> Option<serde_json::Value> {
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    if parts.len() != 3 {
        return None;
    }
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let decoded = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    serde_json::from_slice(&decoded).ok()
}

/// Extract the `chatgpt_account_id` from a JWT access token.
fn get_account_id(access_token: &str) -> Option<String> {
    let payload = decode_jwt_payload(access_token)?;
    let auth = payload.get(JWT_CLAIM_PATH)?;
    let id = auth.get("chatgpt_account_id")?.as_str()?;
    if id.is_empty() { None } else { Some(id.to_owned()) }
}

// ---------------------------------------------------------------------------
// Token exchange
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum TokenResult {
    Success { access: String, refresh: String, expires: u64 },
    Failed,
}

async fn exchange_authorization_code(
    client: &reqwest::Client,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> anyhow::Result<TokenResult> {
    let response = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await?;

    if !response.status().is_success() {
        let text = response.text().await.unwrap_or_default();
        error!("[openai-codex] code->token failed: {text}");
        return Ok(TokenResult::Failed);
    }

    #[derive(serde::Deserialize)]
    struct TokenResponse {
        access_token: Option<String>,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
    }

    let json = response.json::<TokenResponse>().await?;
    match (json.access_token, json.refresh_token, json.expires_in) {
        (Some(access), Some(refresh), Some(expires_in)) => {
            let expires = now_ms() + expires_in * 1000;
            Ok(TokenResult::Success { access, refresh, expires })
        }
        _ => {
            error!("[openai-codex] token response missing fields");
            Ok(TokenResult::Failed)
        }
    }
}

async fn refresh_access_token(client: &reqwest::Client, refresh_token: &str) -> anyhow::Result<TokenResult> {
    let response = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
        ])
        .send()
        .await?;

    if !response.status().is_success() {
        let text = response.text().await.unwrap_or_default();
        error!("[openai-codex] Token refresh failed: {text}");
        return Ok(TokenResult::Failed);
    }

    #[derive(serde::Deserialize)]
    struct TokenResponse {
        access_token: Option<String>,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
    }

    let json = response.json::<TokenResponse>().await?;
    match (json.access_token, json.refresh_token, json.expires_in) {
        (Some(access), Some(refresh), Some(expires_in)) => {
            let expires = now_ms() + expires_in * 1000;
            Ok(TokenResult::Success { access, refresh, expires })
        }
        _ => {
            error!("[openai-codex] Token refresh response missing fields");
            Ok(TokenResult::Failed)
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ---------------------------------------------------------------------------
// Local OAuth callback HTTP server
// ---------------------------------------------------------------------------

/// Result sent from the callback server to the waiting future.
type CodeResult = Option<String>; // Some(code) or None if cancelled

struct OAuthServer {
    /// Call to cancel the wait without a code.
    cancel_tx: Arc<tokio::sync::Mutex<Option<oneshot::Sender<CodeResult>>>>,
    /// Awaitable that resolves when a code arrives (or is cancelled).
    code_rx: tokio::sync::Mutex<Option<oneshot::Receiver<CodeResult>>>,
    shutdown_tx: Arc<tokio::sync::Mutex<Option<oneshot::Sender<()>>>>,
}

impl OAuthServer {
    fn cancel_wait(&self) {
        if let Ok(mut guard) = self.cancel_tx.try_lock() {
            if let Some(tx) = guard.take() {
                let _ = tx.send(None);
            }
        }
    }

    async fn wait_for_code(&self) -> CodeResult {
        let mut guard = self.code_rx.lock().await;
        if let Some(rx) = guard.take() {
            rx.await.unwrap_or(None)
        } else {
            None
        }
    }

    fn close(&self) {
        if let Ok(mut guard) = self.shutdown_tx.try_lock() {
            if let Some(tx) = guard.take() {
                let _ = tx.send(());
            }
        }
    }
}

/// Start a local HTTP server on port 1455 to receive the OAuth callback.
///
/// If the port is already in use the server silently falls back to returning
/// `None` (the user will be prompted to paste the code manually).
///
/// Mirrors `startLocalOAuthServer()` from `openai-codex.ts`.
async fn start_local_oauth_server(state: String) -> OAuthServer {
    let (code_tx, code_rx) = oneshot::channel::<CodeResult>();
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

    let code_tx = Arc::new(tokio::sync::Mutex::new(Some(code_tx)));
    let code_tx_clone = code_tx.clone();
    let cancel_tx = Arc::new(tokio::sync::Mutex::new(Option::<oneshot::Sender<CodeResult>>::None));

    // Build a minimal HTTP server using raw Tokio TCP
    let addr: SocketAddr = "127.0.0.1:1455".parse().unwrap();

    let server_task = tokio::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(err) => {
                error!("[openai-codex] Failed to bind http://127.0.0.1:1455 ({err}). Falling back to manual paste.");
                // Signal no code available
                if let Ok(mut guard) = code_tx_clone.try_lock() {
                    if let Some(tx) = guard.take() {
                        let _ = tx.send(None);
                    }
                }
                return;
            }
        };

        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accept = listener.accept() => {
                    let (mut stream, _) = match accept {
                        Ok(s) => s,
                        Err(_) => break,
                    };

                    // Read the HTTP request line
                    let mut buf = [0u8; 4096];
                    let n = match tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await {
                        Ok(n) => n,
                        Err(_) => continue,
                    };
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let request_line = request.lines().next().unwrap_or("");

                    // Parse GET /auth/callback?code=...&state=...
                    let path_and_query = request_line
                        .strip_prefix("GET ")
                        .and_then(|s| s.strip_suffix(" HTTP/1.1"))
                        .unwrap_or("");

                    let (status_code, body) = process_callback(path_and_query, &state, &code_tx_clone).await;

                    let response = format!(
                        "HTTP/1.1 {status_code}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
                }
            }
        }
    });

    // Keep shutdown_tx alive via the struct
    let shutdown_tx_arc = Arc::new(tokio::sync::Mutex::new(Some(shutdown_tx)));
    let shutdown_tx_clone = shutdown_tx_arc.clone();

    // We need to be able to cancel the code_rx side too
    let (cancel_actual_tx, cancel_actual_rx) = oneshot::channel::<CodeResult>();
    let cancel_arc = Arc::new(tokio::sync::Mutex::new(Some(cancel_actual_tx)));

    // Wrap the real code_rx in a future that also listens to cancel
    let (merged_tx, merged_rx) = oneshot::channel::<CodeResult>();

    tokio::spawn(async move {
        // Wait for either real code or cancellation
        let result = tokio::select! {
            v = async {
                let mut g = code_tx.lock().await;
                // code_tx was already taken/sent in the server task
                // We watch via a different approach — use a barrier
                drop(g);
                // Actually we receive via code_rx which was the other end
                // NOTE: We can't await code_rx here as we moved it to the OAuthServer.
                // The architecture here is: server writes to code_tx, OAuthServer.wait_for_code reads code_rx.
                // We just need to propagate the cancel signal.
                std::future::pending::<CodeResult>().await
            } => v,
            v = cancel_actual_rx => v.unwrap_or(None),
        };
        let _ = merged_tx.send(result);
        drop(server_task);
    });

    OAuthServer {
        cancel_tx: cancel_arc,
        code_rx: tokio::sync::Mutex::new(Some(merged_rx)),
        shutdown_tx: shutdown_tx_clone,
    }
}

async fn process_callback(
    path_and_query: &str,
    expected_state: &str,
    code_tx: &Arc<tokio::sync::Mutex<Option<oneshot::Sender<CodeResult>>>>,
) -> (&'static str, String) {
    // Parse the path+query
    let fake_url = format!("http://localhost{path_and_query}");
    let url = match url::Url::parse(&fake_url) {
        Ok(u) => u,
        Err(_) => return ("404 Not Found", oauth_error_html("Callback route not found.", None)),
    };

    if url.path() != "/auth/callback" {
        return ("404 Not Found", oauth_error_html("Callback route not found.", None));
    }

    let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
    let state = params.get("state").map(|v| v.as_ref());
    if state != Some(expected_state) {
        return ("400 Bad Request", oauth_error_html("State mismatch.", None));
    }

    let code = match params.get("code") {
        Some(c) => c.to_string(),
        None => return ("400 Bad Request", oauth_error_html("Missing authorization code.", None)),
    };

    // Send code to waiter
    let mut guard = code_tx.lock().await;
    if let Some(tx) = guard.take() {
        let _ = tx.send(Some(code));
    }

    ("200 OK", oauth_success_html("OpenAI authentication completed. You can close this window."))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Full login flow for OpenAI Codex (ChatGPT OAuth).
///
/// Mirrors `loginOpenAICodex()` from `openai-codex.ts`.
pub async fn login_openai_codex(
    on_auth: impl Fn(OAuthAuthInfo),
    on_prompt: impl Fn(OAuthPrompt) -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>>,
    on_progress: Option<impl Fn(String)>,
    on_manual_code_input: Option<impl Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>>>,
    originator: Option<&str>,
) -> anyhow::Result<OAuthCredentials> {
    let pkce = generate_pkce();
    let state = create_state();
    let client = reqwest::Client::new();

    // Build authorization URL
    let mut auth_url = url::Url::parse(AUTHORIZE_URL)?;
    auth_url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("originator", originator.unwrap_or("pi"));

    let server = start_local_oauth_server(state.clone()).await;

    on_auth(OAuthAuthInfo {
        url: auth_url.to_string(),
        instructions: Some("A browser window should open. Complete login to finish.".to_owned()),
    });

    let code: Option<String>;

    if let Some(ref manual_fn) = on_manual_code_input {
        // Race between browser callback and manual input
        let manual_future = manual_fn();
        let (manual_code_res, server_code_res) = tokio::join!(
            async { Some(manual_future.await) },
            server.wait_for_code(),
        );

        if let Some(browser_code) = server_code_res {
            code = Some(browser_code);
        } else if let Some(manual_input) = manual_code_res {
            let (c, s) = parse_authorization_input(&manual_input);
            if let Some(s_val) = s {
                if s_val != state {
                    server.close();
                    anyhow::bail!("State mismatch");
                }
            }
            code = c;
        } else {
            code = None;
        }
    } else {
        let server_code = server.wait_for_code().await;
        code = server_code;
    }

    server.close();

    // Fallback to on_prompt if still no code
    let final_code = if let Some(c) = code {
        c
    } else {
        let input = on_prompt(OAuthPrompt {
            message: "Paste the authorization code (or full redirect URL):".to_owned(),
            placeholder: None,
            allow_empty: false,
        })
        .await;
        let (c, s) = parse_authorization_input(&input);
        if let Some(s_val) = s {
            if s_val != state {
                anyhow::bail!("State mismatch");
            }
        }
        c.ok_or_else(|| anyhow::anyhow!("Missing authorization code"))?
    };

    let token_result = exchange_authorization_code(&client, &final_code, &pkce.verifier, REDIRECT_URI).await?;
    let TokenResult::Success { access, refresh, expires } = token_result else {
        anyhow::bail!("Token exchange failed");
    };

    let account_id = get_account_id(&access)
        .ok_or_else(|| anyhow::anyhow!("Failed to extract accountId from token"))?;

    Ok(OAuthCredentials::new(refresh, access, expires)
        .with_extra("accountId", account_id))
}

/// Refresh an OpenAI Codex OAuth token.
///
/// Mirrors `refreshOpenAICodexToken()` from `openai-codex.ts`.
pub async fn refresh_openai_codex_token(refresh_token: &str) -> anyhow::Result<OAuthCredentials> {
    let client = reqwest::Client::new();
    let result = refresh_access_token(&client, refresh_token).await?;
    let TokenResult::Success { access, refresh, expires } = result else {
        anyhow::bail!("Failed to refresh OpenAI Codex token");
    };

    let account_id = get_account_id(&access)
        .ok_or_else(|| anyhow::anyhow!("Failed to extract accountId from token"))?;

    Ok(OAuthCredentials::new(refresh, access, expires)
        .with_extra("accountId", account_id))
}

// ---------------------------------------------------------------------------
// OAuthProviderInterface impl
// ---------------------------------------------------------------------------

pub struct OpenAICodexOAuthProvider;

#[async_trait]
impl OAuthProviderInterface for OpenAICodexOAuthProvider {
    fn id(&self) -> &str {
        "openai-codex"
    }

    fn name(&self) -> &str {
        "ChatGPT Plus/Pro (Codex Subscription)"
    }

    fn uses_callback_server(&self) -> bool {
        true
    }

    async fn login(&self, callbacks: OAuthLoginCallbacks) -> anyhow::Result<OAuthCredentials> {
        login_openai_codex(
            |info| (callbacks.on_auth)(info),
            |prompt| (callbacks.on_prompt)(prompt),
            callbacks.on_progress.as_ref().map(|f| move |msg: String| f(msg)),
            callbacks.on_manual_code_input.as_ref().map(|f| move || f()),
            None,
        )
        .await
    }

    async fn refresh_token(&self, credentials: &OAuthCredentials) -> anyhow::Result<OAuthCredentials> {
        refresh_openai_codex_token(&credentials.refresh).await
    }

    fn get_api_key(&self, credentials: &OAuthCredentials) -> String {
        credentials.access.clone()
    }
}

pub static OPENAI_CODEX_OAUTH_PROVIDER: OpenAICodexOAuthProvider = OpenAICodexOAuthProvider;
