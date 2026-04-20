//! OAuth shared types — Rust counterpart of `packages/ai/src/utils/oauth/types.ts`.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Persisted OAuth credentials for a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthCredentials {
    /// OAuth refresh token (used to obtain new access tokens).
    pub refresh: String,
    /// Current access token.
    pub access: String,
    /// Expiry timestamp in milliseconds since Unix epoch.
    pub expires: u64,
    /// Provider-specific extra fields (e.g. `projectId`, `enterpriseUrl`).
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl OAuthCredentials {
    pub fn new(refresh: impl Into<String>, access: impl Into<String>, expires: u64) -> Self {
        Self {
            refresh: refresh.into(),
            access: access.into(),
            expires,
            extra: HashMap::new(),
        }
    }

    pub fn with_extra(
        mut self,
        key: impl Into<String>,
        value: impl Into<serde_json::Value>,
    ) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }

    /// Convenience: get a string extra field.
    pub fn extra_str(&self, key: &str) -> Option<&str> {
        self.extra.get(key)?.as_str()
    }
}

/// A unique string identifier for an OAuth provider.
pub type OAuthProviderId = String;

/// Prompt shown to the user requesting text input.
#[derive(Debug, Clone)]
pub struct OAuthPrompt {
    pub message: String,
    pub placeholder: Option<String>,
    pub allow_empty: bool,
}

/// Auth URL/instructions passed to the `on_auth` callback.
#[derive(Debug, Clone)]
pub struct OAuthAuthInfo {
    pub url: String,
    pub instructions: Option<String>,
}

/// Callbacks required during the OAuth login flow.
///
/// Mirrors `OAuthLoginCallbacks` from `types.ts`.
/// In Rust the callbacks are boxed async closures / futures rather than
/// JS-style function objects.
pub struct OAuthLoginCallbacks {
    /// Called when the provider has an auth URL ready for the user.
    pub on_auth: Box<dyn Fn(OAuthAuthInfo) + Send + Sync>,
    /// Called to prompt the user for input; returns the entered string.
    pub on_prompt: Box<
        dyn Fn(OAuthPrompt) -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>>
            + Send
            + Sync,
    >,
    /// Optional progress message callback.
    pub on_progress: Option<Box<dyn Fn(String) + Send + Sync>>,
    /// Optional: race with browser callback — resolves with manually-pasted code.
    pub on_manual_code_input: Option<
        Box<
            dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>>
                + Send
                + Sync,
        >,
    >,
}

/// Trait implemented by every OAuth provider.
///
/// Mirrors `OAuthProviderInterface` from `types.ts`.
#[async_trait]
pub trait OAuthProviderInterface: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;

    /// Whether the login flow uses a local callback HTTP server and supports
    /// manual code input as a fallback.
    fn uses_callback_server(&self) -> bool {
        false
    }

    /// Run the full login flow; returns credentials to persist.
    async fn login(&self, callbacks: OAuthLoginCallbacks) -> anyhow::Result<OAuthCredentials>;

    /// Refresh expired credentials; returns updated credentials to persist.
    async fn refresh_token(
        &self,
        credentials: &OAuthCredentials,
    ) -> anyhow::Result<OAuthCredentials>;

    /// Convert credentials to the API key string expected by this provider.
    fn get_api_key(&self, credentials: &OAuthCredentials) -> String;

    /// Optionally rewrite model base URLs after credentials are resolved.
    ///
    /// Default implementation returns models unchanged. Providers that derive
    /// their base URL from credentials (e.g. GitHub Copilot's proxy endpoint)
    /// should override this.
    fn modify_models(
        &self,
        models: Vec<crate::types::Model>,
        _credentials: &OAuthCredentials,
    ) -> Vec<crate::types::Model> {
        models
    }
}
