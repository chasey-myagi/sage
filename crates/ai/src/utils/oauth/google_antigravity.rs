//! Google Antigravity OAuth flow (Gemini 3, Claude, GPT-OSS via Google Cloud).
//! Rust counterpart of `packages/ai/src/utils/oauth/google-antigravity.ts`.
//!
//! Uses PKCE + local HTTP callback server on port 51121. CLI use only.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::oneshot;
use super::oauth_page::{oauth_error_html, oauth_success_html};
use super::pkce::generate_pkce;
use super::types::{OAuthAuthInfo, OAuthCredentials, OAuthLoginCallbacks, OAuthProviderInterface};

// Antigravity OAuth credentials — obtain from Google Cloud Console and set at build time.
// Override via GOOGLE_ANTIGRAVITY_CLIENT_ID / GOOGLE_ANTIGRAVITY_CLIENT_SECRET env vars.
const CLIENT_ID: &str = "";
const CLIENT_SECRET: &str = "";
const REDIRECT_URI: &str = "http://localhost:51121/oauth-callback";

const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/cloud-platform",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
    "https://www.googleapis.com/auth/cclog",
    "https://www.googleapis.com/auth/experimentsandconfigs",
];

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const DEFAULT_PROJECT_ID: &str = "rising-fact-p41fc";

// ---------------------------------------------------------------------------
// Local callback server
// ---------------------------------------------------------------------------

struct CallbackServer {
    code_rx: tokio::sync::Mutex<Option<oneshot::Receiver<Option<(String, String)>>>>,
    cancel_tx: Arc<tokio::sync::Mutex<Option<oneshot::Sender<Option<(String, String)>>>>>,
    shutdown_tx: Arc<tokio::sync::Mutex<Option<oneshot::Sender<()>>>>,
}

impl CallbackServer {
    fn cancel_wait(&self) {
        if let Ok(mut g) = self.cancel_tx.try_lock() {
            if let Some(tx) = g.take() {
                let _ = tx.send(None);
            }
        }
    }

    async fn wait_for_code(&self) -> Option<(String, String)> {
        let mut g = self.code_rx.lock().await;
        if let Some(rx) = g.take() {
            rx.await.unwrap_or(None)
        } else {
            None
        }
    }

    fn close(&self) {
        if let Ok(mut g) = self.shutdown_tx.try_lock() {
            if let Some(tx) = g.take() {
                let _ = tx.send(());
            }
        }
    }
}

async fn start_callback_server() -> anyhow::Result<CallbackServer> {
    let addr: SocketAddr = "127.0.0.1:51121".parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await
        .map_err(|e| anyhow::anyhow!("Failed to bind port 51121: {e}"))?;

    let (code_tx, code_rx) = oneshot::channel::<Option<(String, String)>>();
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

    let code_tx = Arc::new(tokio::sync::Mutex::new(Some(code_tx)));
    let code_tx_srv = code_tx.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accept = listener.accept() => {
                    let (mut stream, _) = match accept {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    let mut buf = [0u8; 4096];
                    let n = match tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await {
                        Ok(n) => n,
                        Err(_) => continue,
                    };
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let path_and_query = request
                        .lines()
                        .next()
                        .and_then(|l| l.strip_prefix("GET "))
                        .and_then(|l| l.strip_suffix(" HTTP/1.1"))
                        .unwrap_or("")
                        .to_owned();

                    let (status, body) = handle_callback(&path_and_query, &code_tx_srv).await;
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
                }
            }
        }
    });

    let shutdown_arc = Arc::new(tokio::sync::Mutex::new(Some(shutdown_tx)));
    let cancel_arc = Arc::new(tokio::sync::Mutex::new(Option::<oneshot::Sender<Option<(String, String)>>>::None));

    // We re-use code_tx as our cancel mechanism: sending None cancels waiting
    Ok(CallbackServer {
        code_rx: tokio::sync::Mutex::new(Some(code_rx)),
        cancel_tx: code_tx, // send None to cancel
        shutdown_tx: shutdown_arc,
    })
}

async fn handle_callback(
    path_and_query: &str,
    code_tx: &Arc<tokio::sync::Mutex<Option<oneshot::Sender<Option<(String, String)>>>>>,
) -> (&'static str, String) {
    let fake_url = format!("http://localhost{path_and_query}");
    let url = match url::Url::parse(&fake_url) {
        Ok(u) => u,
        Err(_) => return ("404 Not Found", oauth_error_html("Callback route not found.", None)),
    };

    if url.path() != "/oauth-callback" {
        return ("404 Not Found", oauth_error_html("Callback route not found.", None));
    }

    let params: std::collections::HashMap<_, _> = url.query_pairs().collect();

    if let Some(error) = params.get("error") {
        return (
            "400 Bad Request",
            oauth_error_html(
                "Google authentication did not complete.",
                Some(&format!("Error: {error}")),
            ),
        );
    }

    match (params.get("code"), params.get("state")) {
        (Some(code), Some(state)) => {
            let mut guard = code_tx.lock().await;
            if let Some(tx) = guard.take() {
                let _ = tx.send(Some((code.to_string(), state.to_string())));
            }
            (
                "200 OK",
                oauth_success_html("Google authentication completed. You can close this window."),
            )
        }
        _ => ("400 Bad Request", oauth_error_html("Missing code or state parameter.", None)),
    }
}

// ---------------------------------------------------------------------------
// Redirect URL parser
// ---------------------------------------------------------------------------

fn parse_redirect_url(input: &str) -> (Option<String>, Option<String>) {
    let value = input.trim();
    if value.is_empty() {
        return (None, None);
    }
    match url::Url::parse(value) {
        Ok(url) => {
            let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
            let code = params.get("code").map(|v| v.to_string());
            let state = params.get("state").map(|v| v.to_string());
            (code, state)
        }
        Err(_) => (None, None),
    }
}

// ---------------------------------------------------------------------------
// Project discovery
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoadCodeAssistPayload {
    #[serde(default)]
    cloudaicompanion_project: Option<serde_json::Value>,
}

/// Returns the discovered project ID. Progress messages are emitted via two
/// optional string slices so no non-`Send` closure reference is held across
/// `.await` points.
async fn discover_project(access_token: &str) -> (String, bool) {
    // Returns (project_id, used_default)
    let headers = {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {access_token}").parse().unwrap(),
        );
        h.insert(reqwest::header::CONTENT_TYPE, "application/json".parse().unwrap());
        h.insert("User-Agent", "google-api-nodejs-client/9.15.1".parse().unwrap());
        h.insert("X-Goog-Api-Client", "google-cloud-sdk vscode_cloudshelleditor/0.1".parse().unwrap());
        h.insert(
            "Client-Metadata",
            serde_json::json!({
                "ideType": "IDE_UNSPECIFIED",
                "platform": "PLATFORM_UNSPECIFIED",
                "pluginType": "GEMINI"
            })
            .to_string()
            .parse()
            .unwrap(),
        );
        h
    };

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "metadata": {
            "ideType": "IDE_UNSPECIFIED",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI"
        }
    });

    let endpoints = [
        "https://cloudcode-pa.googleapis.com",
        "https://daily-cloudcode-pa.sandbox.googleapis.com",
    ];

    for endpoint in &endpoints {
        let url = format!("{endpoint}/v1internal:loadCodeAssist");
        match client.post(&url).headers(headers.clone()).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(data) = resp.json::<LoadCodeAssistPayload>().await {
                    match data.cloudaicompanion_project {
                        Some(serde_json::Value::String(s)) if !s.is_empty() => return (s, false),
                        Some(serde_json::Value::Object(ref map)) => {
                            if let Some(serde_json::Value::String(id)) = map.get("id") {
                                if !id.is_empty() {
                                    return (id.clone(), false);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    (DEFAULT_PROJECT_ID.to_owned(), true)
}

async fn get_user_email(access_token: &str) -> Option<String> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://www.googleapis.com/oauth2/v1/userinfo?alt=json")
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    #[derive(Deserialize)]
    struct UserInfo {
        email: Option<String>,
    }

    resp.json::<UserInfo>().await.ok()?.email
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Refresh an Antigravity access token.
///
/// Mirrors `refreshAntigravityToken()` from `google-antigravity.ts`.
pub async fn refresh_antigravity_token(refresh_token: &str, project_id: &str) -> anyhow::Result<OAuthCredentials> {
    let client = reqwest::Client::new();
    let response = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await?;

    if !response.status().is_success() {
        let error = response.text().await.unwrap_or_default();
        anyhow::bail!("Antigravity token refresh failed: {error}");
    }

    #[derive(Deserialize)]
    struct TokenData {
        access_token: String,
        expires_in: u64,
        #[serde(default)]
        refresh_token: Option<String>,
    }

    let data = response.json::<TokenData>().await?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let expires = now_ms + data.expires_in * 1000 - 5 * 60 * 1000;
    let new_refresh = data.refresh_token.as_deref().unwrap_or(refresh_token);

    Ok(OAuthCredentials::new(new_refresh, &data.access_token, expires)
        .with_extra("projectId", project_id))
}

/// Full login flow for Antigravity OAuth.
///
/// Mirrors `loginAntigravity()` from `google-antigravity.ts`.
pub async fn login_antigravity(
    on_auth: impl Fn(OAuthAuthInfo),
    on_progress: Option<impl Fn(String)>,
    on_manual_code_input: Option<impl Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>>>,
) -> anyhow::Result<OAuthCredentials> {
    let pkce = generate_pkce();
    let progress = |msg: &str| {
        if let Some(ref f) = on_progress {
            f(msg.to_owned());
        }
    };

    progress("Starting local server for OAuth callback...");
    let server = start_callback_server().await?;

    let code: Option<String>;

    // Build authorization URL (use verifier as state, matching TS source)
    let state = pkce.verifier.clone();
    let mut auth_url = url::Url::parse(AUTH_URL)?;
    auth_url
        .query_pairs_mut()
        .append_pair("client_id", CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", &SCOPES.join(" "))
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");

    on_auth(OAuthAuthInfo {
        url: auth_url.to_string(),
        instructions: Some("Complete the sign-in in your browser.".to_owned()),
    });

    progress("Waiting for OAuth callback...");

    if let Some(ref manual_fn) = on_manual_code_input {
        let manual_future = manual_fn();
        let (manual_res, server_res) = tokio::join!(
            async { Some(manual_future.await) },
            server.wait_for_code(),
        );

        if let Some((browser_code, browser_state)) = server_res {
            if browser_state != state {
                server.close();
                anyhow::bail!("OAuth state mismatch - possible CSRF attack");
            }
            code = Some(browser_code);
        } else if let Some(manual_input) = manual_res {
            let (c, s) = parse_redirect_url(&manual_input);
            if let Some(s_val) = s {
                if s_val != state {
                    server.close();
                    anyhow::bail!("OAuth state mismatch - possible CSRF attack");
                }
            }
            code = c;
        } else {
            code = None;
        }
    } else {
        let result = server.wait_for_code().await;
        if let Some((browser_code, browser_state)) = result {
            if browser_state != state {
                server.close();
                anyhow::bail!("OAuth state mismatch - possible CSRF attack");
            }
            code = Some(browser_code);
        } else {
            code = None;
        }
    }

    server.close();

    let final_code = code.ok_or_else(|| anyhow::anyhow!("No authorization code received"))?;

    // Exchange code for tokens
    progress("Exchanging authorization code for tokens...");
    let client = reqwest::Client::new();
    let token_response = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("code", final_code.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", pkce.verifier.as_str()),
        ])
        .send()
        .await?;

    if !token_response.status().is_success() {
        let error = token_response.text().await.unwrap_or_default();
        anyhow::bail!("Token exchange failed: {error}");
    }

    #[derive(Deserialize)]
    struct TokenData {
        access_token: String,
        expires_in: u64,
        refresh_token: Option<String>,
    }

    let token_data = token_response.json::<TokenData>().await?;
    let refresh_token = token_data
        .refresh_token
        .ok_or_else(|| anyhow::anyhow!("No refresh token received. Please try again."))?;

    progress("Getting user info...");
    let email = get_user_email(&token_data.access_token).await;

    progress("Checking for existing project...");
    let (project_id, used_default) = discover_project(&token_data.access_token).await;
    if used_default {
        progress("Using default project...");
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let expires = now_ms + token_data.expires_in * 1000 - 5 * 60 * 1000;

    let mut creds = OAuthCredentials::new(refresh_token, &token_data.access_token, expires)
        .with_extra("projectId", &*project_id);

    if let Some(email_val) = email {
        creds = creds.with_extra("email", email_val);
    }

    Ok(creds)
}

// ---------------------------------------------------------------------------
// OAuthProviderInterface impl
// ---------------------------------------------------------------------------

pub struct AntigravityOAuthProvider;

#[async_trait]
impl OAuthProviderInterface for AntigravityOAuthProvider {
    fn id(&self) -> &str {
        "google-antigravity"
    }

    fn name(&self) -> &str {
        "Antigravity (Gemini 3, Claude, GPT-OSS)"
    }

    fn uses_callback_server(&self) -> bool {
        true
    }

    async fn login(&self, callbacks: OAuthLoginCallbacks) -> anyhow::Result<OAuthCredentials> {
        login_antigravity(
            |info| (callbacks.on_auth)(info),
            callbacks.on_progress.as_ref().map(|f| move |msg: String| f(msg)),
            callbacks.on_manual_code_input.as_ref().map(|f| move || f()),
        )
        .await
    }

    async fn refresh_token(&self, credentials: &OAuthCredentials) -> anyhow::Result<OAuthCredentials> {
        let project_id = credentials
            .extra_str("projectId")
            .ok_or_else(|| anyhow::anyhow!("Antigravity credentials missing projectId"))?
            .to_owned();
        refresh_antigravity_token(&credentials.refresh, &project_id).await
    }

    fn get_api_key(&self, credentials: &OAuthCredentials) -> String {
        let project_id = credentials.extra_str("projectId").unwrap_or_default();
        serde_json::json!({
            "token": credentials.access,
            "projectId": project_id,
        })
        .to_string()
    }
}

pub static ANTIGRAVITY_OAUTH_PROVIDER: AntigravityOAuthProvider = AntigravityOAuthProvider;
