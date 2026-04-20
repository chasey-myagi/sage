//! OAuth credential management for AI providers.
//! Rust counterpart of `packages/ai/src/utils/oauth/index.ts`.
//!
//! Handles login, token refresh, and credential storage for OAuth-based
//! providers: GitHub Copilot, OpenAI Codex, and Google Antigravity.

pub mod github_copilot;
pub mod google_antigravity;
pub mod oauth_page;
pub mod openai_codex;
pub mod pkce;
pub mod types;

// Re-export the most commonly used items.
pub use github_copilot::{
    GITHUB_COPILOT_OAUTH_PROVIDER, GitHubCopilotOAuthProvider, get_github_copilot_base_url,
    login_github_copilot, normalize_domain, refresh_github_copilot_token,
};
pub use google_antigravity::{
    ANTIGRAVITY_OAUTH_PROVIDER, AntigravityOAuthProvider, login_antigravity,
    refresh_antigravity_token,
};
pub use oauth_page::{oauth_error_html, oauth_success_html};
pub use openai_codex::{
    OPENAI_CODEX_OAUTH_PROVIDER, OpenAICodexOAuthProvider, login_openai_codex,
    refresh_openai_codex_token,
};
pub use pkce::{Pkce, generate_pkce};
pub use types::{
    OAuthAuthInfo, OAuthCredentials, OAuthLoginCallbacks, OAuthPrompt, OAuthProviderId,
    OAuthProviderInterface,
};

// ============================================================================
// Provider registry
// ============================================================================

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use std::sync::LazyLock;

static OAUTH_PROVIDER_REGISTRY: LazyLock<RwLock<HashMap<String, Arc<dyn OAuthProviderInterface>>>> =
    LazyLock::new(|| {
        let mut map: HashMap<String, Arc<dyn OAuthProviderInterface>> = HashMap::new();
        map.insert(
            "github-copilot".to_owned(),
            Arc::new(GitHubCopilotOAuthProvider),
        );
        map.insert(
            "openai-codex".to_owned(),
            Arc::new(OpenAICodexOAuthProvider),
        );
        map.insert(
            "google-antigravity".to_owned(),
            Arc::new(AntigravityOAuthProvider),
        );
        RwLock::new(map)
    });

/// Get a registered OAuth provider by its ID.
///
/// Mirrors `getOAuthProvider()` from `index.ts`.
pub fn get_oauth_provider(id: &str) -> Option<Arc<dyn OAuthProviderInterface>> {
    OAUTH_PROVIDER_REGISTRY.read().ok()?.get(id).cloned()
}

/// Register a custom OAuth provider (or override an existing one).
///
/// Mirrors `registerOAuthProvider()` from `index.ts`.
pub fn register_oauth_provider(provider: Arc<dyn OAuthProviderInterface>) {
    if let Ok(mut map) = OAUTH_PROVIDER_REGISTRY.write() {
        map.insert(provider.id().to_owned(), provider);
    }
}

/// Unregister an OAuth provider. Built-in providers are restored to their
/// default implementations; custom providers are removed entirely.
///
/// Mirrors `unregisterOAuthProvider()` from `index.ts`.
pub fn unregister_oauth_provider(id: &str) {
    if let Ok(mut map) = OAUTH_PROVIDER_REGISTRY.write() {
        let builtin: Option<Arc<dyn OAuthProviderInterface>> = match id {
            "github-copilot" => Some(Arc::new(GitHubCopilotOAuthProvider)),
            "openai-codex" => Some(Arc::new(OpenAICodexOAuthProvider)),
            "google-antigravity" => Some(Arc::new(AntigravityOAuthProvider)),
            _ => None,
        };
        match builtin {
            Some(p) => {
                map.insert(id.to_owned(), p);
            }
            None => {
                map.remove(id);
            }
        }
    }
}

/// Reset the registry to only the built-in providers.
///
/// Mirrors `resetOAuthProviders()` from `index.ts`.
pub fn reset_oauth_providers() {
    if let Ok(mut map) = OAUTH_PROVIDER_REGISTRY.write() {
        map.clear();
        map.insert(
            "github-copilot".to_owned(),
            Arc::new(GitHubCopilotOAuthProvider),
        );
        map.insert(
            "openai-codex".to_owned(),
            Arc::new(OpenAICodexOAuthProvider),
        );
        map.insert(
            "google-antigravity".to_owned(),
            Arc::new(AntigravityOAuthProvider),
        );
    }
}

/// Return all registered OAuth providers.
///
/// Mirrors `getOAuthProviders()` from `index.ts`.
pub fn get_oauth_providers() -> Vec<Arc<dyn OAuthProviderInterface>> {
    OAUTH_PROVIDER_REGISTRY
        .read()
        .map(|map| map.values().cloned().collect())
        .unwrap_or_default()
}

// ============================================================================
// High-level API
// ============================================================================

/// Refresh the token for any registered OAuth provider.
///
/// Mirrors `refreshOAuthToken()` from `index.ts`.
pub async fn refresh_oauth_token(
    provider_id: &str,
    credentials: &OAuthCredentials,
) -> anyhow::Result<OAuthCredentials> {
    let provider = get_oauth_provider(provider_id)
        .ok_or_else(|| anyhow::anyhow!("Unknown OAuth provider: {provider_id}"))?;
    provider.refresh_token(credentials).await
}

/// Get the API key for a provider from its stored OAuth credentials.
/// Automatically refreshes the token if it has expired.
///
/// Returns `None` if no credentials are stored for the given provider.
/// Returns `(updated_credentials, api_key)` otherwise.
///
/// Mirrors `getOAuthApiKey()` from `index.ts`.
pub async fn get_oauth_api_key(
    provider_id: &str,
    credentials_map: &HashMap<String, OAuthCredentials>,
) -> anyhow::Result<Option<(OAuthCredentials, String)>> {
    let provider = get_oauth_provider(provider_id)
        .ok_or_else(|| anyhow::anyhow!("Unknown OAuth provider: {provider_id}"))?;

    let Some(creds) = credentials_map.get(provider_id) else {
        return Ok(None);
    };

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let creds = if now_ms >= creds.expires {
        provider
            .refresh_token(creds)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to refresh OAuth token for {provider_id}: {e}"))?
    } else {
        creds.clone()
    };

    let api_key = provider.get_api_key(&creds);
    Ok(Some((creds, api_key)))
}
