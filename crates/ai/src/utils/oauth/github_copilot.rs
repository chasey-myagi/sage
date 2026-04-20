//! GitHub Copilot OAuth flow (device code).
//! Rust counterpart of `packages/ai/src/utils/oauth/github-copilot.ts`.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;
use tracing::warn;

use super::types::{
    OAuthAuthInfo, OAuthCredentials, OAuthLoginCallbacks, OAuthPrompt, OAuthProviderInterface,
};

// CLIENT_ID is the base64-decoded value of "SXYxLmI1MDdhMDhjODdlY2ZlOTg="
// i.e. "Iv1.b507a08c87ecfe98"
const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";

const INITIAL_POLL_INTERVAL_MULTIPLIER: f64 = 1.2;
const SLOW_DOWN_POLL_INTERVAL_MULTIPLIER: f64 = 1.4;

fn copilot_headers() -> reqwest::header::HeaderMap {
    let mut map = reqwest::header::HeaderMap::new();
    map.insert("User-Agent", "GitHubCopilotChat/0.35.0".parse().unwrap());
    map.insert("Editor-Version", "vscode/1.107.0".parse().unwrap());
    map.insert(
        "Editor-Plugin-Version",
        "copilot-chat/0.35.0".parse().unwrap(),
    );
    map.insert("Copilot-Integration-Id", "vscode-chat".parse().unwrap());
    map
}

struct GitHubUrls {
    device_code_url: String,
    access_token_url: String,
    copilot_token_url: String,
}

fn get_urls(domain: &str) -> GitHubUrls {
    GitHubUrls {
        device_code_url: format!("https://{domain}/login/device/code"),
        access_token_url: format!("https://{domain}/login/oauth/access_token"),
        copilot_token_url: format!("https://api.{domain}/copilot_internal/v2/token"),
    }
}

/// Normalise a GitHub Enterprise URL/domain input to just the hostname.
///
/// Mirrors `normalizeDomain()` from `github-copilot.ts`.
pub fn normalize_domain(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let url_str = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    };
    let url = url::Url::parse(&url_str).ok()?;
    Some(url.host_str()?.to_owned())
}

/// Extract the Copilot API base URL from a Copilot short-lived access token.
///
/// Token format: `tid=...;exp=...;proxy-ep=proxy.individual.githubcopilot.com;...`
/// Returns URL like `https://api.individual.githubcopilot.com`.
fn get_base_url_from_token(token: &str) -> Option<String> {
    let proxy_host = token
        .split(';')
        .find_map(|part| part.strip_prefix("proxy-ep="))?;
    let api_host = if let Some(rest) = proxy_host.strip_prefix("proxy.") {
        format!("api.{rest}")
    } else {
        proxy_host.to_owned()
    };
    Some(format!("https://{api_host}"))
}

/// Return the Copilot API base URL for a given token / enterprise domain.
///
/// Mirrors `getGitHubCopilotBaseUrl()` from `github-copilot.ts`.
pub fn get_github_copilot_base_url(token: Option<&str>, enterprise_domain: Option<&str>) -> String {
    if let Some(tok) = token
        && let Some(url) = get_base_url_from_token(tok)
    {
        return url;
    }
    if let Some(domain) = enterprise_domain {
        return format!("https://copilot-api.{domain}");
    }
    "https://api.individual.githubcopilot.com".to_owned()
}

// ---------------------------------------------------------------------------
// Device flow types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    interval: u64,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DeviceTokenResponse {
    Success {
        access_token: String,
    },
    Error {
        error: String,
        #[serde(default)]
        error_description: Option<String>,
        #[serde(default)]
        interval: Option<u64>,
    },
}

// ---------------------------------------------------------------------------
// Device flow helpers
// ---------------------------------------------------------------------------

async fn start_device_flow(
    client: &reqwest::Client,
    domain: &str,
) -> anyhow::Result<DeviceCodeResponse> {
    let urls = get_urls(domain);
    let response = client
        .post(&urls.device_code_url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("User-Agent", "GitHubCopilotChat/0.35.0")
        .form(&[("client_id", CLIENT_ID), ("scope", "read:user")])
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("{status}: {text}");
    }

    Ok(response.json::<DeviceCodeResponse>().await?)
}

async fn poll_for_github_access_token(
    client: &reqwest::Client,
    domain: &str,
    device_code: &str,
    interval_seconds: u64,
    expires_in: u64,
) -> anyhow::Result<String> {
    let urls = get_urls(domain);
    let deadline = Instant::now() + Duration::from_secs(expires_in);
    let mut interval_ms = u64::max(1000, interval_seconds * 1000);
    let mut interval_multiplier = INITIAL_POLL_INTERVAL_MULTIPLIER;
    let mut slow_down_responses = 0u32;

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait_ms = u64::min(
            (interval_ms as f64 * interval_multiplier).ceil() as u64,
            remaining.as_millis() as u64,
        );
        tokio::time::sleep(Duration::from_millis(wait_ms)).await;

        let response = client
            .post(&urls.access_token_url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("User-Agent", "GitHubCopilotChat/0.35.0")
            .form(&[
                ("client_id", CLIENT_ID),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("{status}: {text}");
        }

        let result = response.json::<DeviceTokenResponse>().await?;
        match result {
            DeviceTokenResponse::Success { access_token } => return Ok(access_token),
            DeviceTokenResponse::Error {
                error,
                error_description,
                interval,
            } => {
                if error == "authorization_pending" {
                    continue;
                }
                if error == "slow_down" {
                    slow_down_responses += 1;
                    interval_ms = match interval {
                        Some(i) if i > 0 => i * 1000,
                        _ => u64::max(1000, interval_ms + 5000),
                    };
                    interval_multiplier = SLOW_DOWN_POLL_INTERVAL_MULTIPLIER;
                    continue;
                }
                let suffix = error_description
                    .map(|d| format!(": {d}"))
                    .unwrap_or_default();
                anyhow::bail!("Device flow failed: {error}{suffix}");
            }
        }
    }

    if slow_down_responses > 0 {
        anyhow::bail!(
            "Device flow timed out after one or more slow_down responses. \
             This is often caused by clock drift in WSL or VM environments. \
             Please sync or restart the VM clock and try again."
        );
    }
    anyhow::bail!("Device flow timed out");
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Refresh a GitHub Copilot short-lived access token using the GitHub OAuth
/// access token as the refresh token.
///
/// Mirrors `refreshGitHubCopilotToken()` from `github-copilot.ts`.
pub async fn refresh_github_copilot_token(
    refresh_token: &str,
    enterprise_domain: Option<&str>,
) -> anyhow::Result<OAuthCredentials> {
    let domain = enterprise_domain.unwrap_or("github.com");
    let urls = get_urls(domain);

    let client = reqwest::Client::new();
    let response = client
        .get(&urls.copilot_token_url)
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {refresh_token}"))
        .headers(copilot_headers())
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("Copilot token refresh failed: {status}: {text}");
    }

    #[derive(Deserialize)]
    struct CopilotTokenResponse {
        token: String,
        expires_at: u64,
    }

    let data = response.json::<CopilotTokenResponse>().await?;
    // expires_at is seconds since epoch; subtract 5-minute buffer; store as ms
    let expires_ms = data.expires_at * 1000 - 5 * 60 * 1000;

    let mut creds = OAuthCredentials::new(refresh_token, &data.token, expires_ms);
    if let Some(domain) = enterprise_domain {
        creds = creds.with_extra("enterpriseUrl", domain);
    }
    Ok(creds)
}

/// Enable a single model policy on the user's Copilot account.
async fn enable_github_copilot_model(
    client: &reqwest::Client,
    token: &str,
    model_id: &str,
    enterprise_domain: Option<&str>,
) -> bool {
    let base_url = get_github_copilot_base_url(Some(token), enterprise_domain);
    let url = format!("{base_url}/models/{model_id}/policy");

    let result = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .headers(copilot_headers())
        .header("openai-intent", "chat-policy")
        .header("x-interaction-type", "chat-policy")
        .json(&serde_json::json!({ "state": "enabled" }))
        .send()
        .await;

    match result {
        Ok(resp) => resp.status().is_success(),
        Err(err) => {
            warn!("Failed to enable Copilot model {model_id}: {err}");
            false
        }
    }
}

/// Full login flow for GitHub Copilot using device code OAuth.
///
/// Mirrors `loginGitHubCopilot()` from `github-copilot.ts`.
pub async fn login_github_copilot(
    on_auth: impl Fn(OAuthAuthInfo),
    on_prompt: impl Fn(
        OAuthPrompt,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>>,
    on_progress: Option<impl Fn(String)>,
) -> anyhow::Result<OAuthCredentials> {
    // Ask for optional Enterprise domain
    let input = on_prompt(OAuthPrompt {
        message: "GitHub Enterprise URL/domain (blank for github.com)".to_owned(),
        placeholder: Some("company.ghe.com".to_owned()),
        allow_empty: true,
    })
    .await;

    let trimmed = input.trim().to_owned();
    let enterprise_domain = if trimmed.is_empty() {
        None
    } else {
        Some(
            normalize_domain(&trimmed)
                .ok_or_else(|| anyhow::anyhow!("Invalid GitHub Enterprise URL/domain"))?,
        )
    };

    let domain = enterprise_domain.as_deref().unwrap_or("github.com");
    let client = reqwest::Client::new();

    let device = start_device_flow(&client, domain).await?;
    on_auth(OAuthAuthInfo {
        url: device.verification_uri.clone(),
        instructions: Some(format!("Enter code: {}", device.user_code)),
    });

    let github_access_token = poll_for_github_access_token(
        &client,
        domain,
        &device.device_code,
        device.interval,
        device.expires_in,
    )
    .await?;

    let credentials =
        refresh_github_copilot_token(&github_access_token, enterprise_domain.as_deref()).await?;

    // Enable all known models (best-effort, fire-and-forget errors)
    if let Some(ref progress) = on_progress {
        progress("Enabling models...".to_owned());
    }
    let models: Vec<_> = crate::models::list_models()
        .iter()
        .filter(|m| m.provider == "github-copilot")
        .collect();
    let futs: Vec<_> = models
        .iter()
        .map(|m| {
            enable_github_copilot_model(
                &client,
                &credentials.access,
                &m.id,
                enterprise_domain.as_deref(),
            )
        })
        .collect();
    futures::future::join_all(futs).await;

    Ok(credentials)
}

// ---------------------------------------------------------------------------
// OAuthProviderInterface impl
// ---------------------------------------------------------------------------

/// Stateless GitHub Copilot OAuth provider.
pub struct GitHubCopilotOAuthProvider;

#[async_trait]
impl OAuthProviderInterface for GitHubCopilotOAuthProvider {
    fn id(&self) -> &str {
        "github-copilot"
    }

    fn name(&self) -> &str {
        "GitHub Copilot"
    }

    async fn login(&self, callbacks: OAuthLoginCallbacks) -> anyhow::Result<OAuthCredentials> {
        login_github_copilot(
            |info| (callbacks.on_auth)(info),
            |prompt| (callbacks.on_prompt)(prompt),
            callbacks
                .on_progress
                .as_ref()
                .map(|f| move |msg: String| f(msg)),
        )
        .await
    }

    async fn refresh_token(
        &self,
        credentials: &OAuthCredentials,
    ) -> anyhow::Result<OAuthCredentials> {
        let enterprise_domain = credentials.extra_str("enterpriseUrl").map(str::to_owned);
        refresh_github_copilot_token(&credentials.refresh, enterprise_domain.as_deref()).await
    }

    fn get_api_key(&self, credentials: &OAuthCredentials) -> String {
        credentials.access.clone()
    }

    fn modify_models(
        &self,
        models: Vec<crate::types::Model>,
        credentials: &OAuthCredentials,
    ) -> Vec<crate::types::Model> {
        let enterprise_domain = credentials
            .extra_str("enterpriseUrl")
            .and_then(normalize_domain);
        let base_url =
            get_github_copilot_base_url(Some(&credentials.access), enterprise_domain.as_deref());
        models
            .into_iter()
            .map(|mut m| {
                if m.provider == "github-copilot" {
                    m.base_url = base_url.clone();
                }
                m
            })
            .collect()
    }
}

/// Process-global singleton instance.
pub static GITHUB_COPILOT_OAUTH_PROVIDER: GitHubCopilotOAuthProvider = GitHubCopilotOAuthProvider;
