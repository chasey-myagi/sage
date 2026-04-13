// Provider registry — matches pi-mono's stream.ts architecture.
// Thread-safe global registry of API providers keyed by their API identifier.

use super::types::*;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

// ---------------------------------------------------------------------------
// CacheRetention
// ---------------------------------------------------------------------------

/// Hint for how long the provider should retain cached prefixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheRetention {
    /// Standard TTL (provider default).
    Standard,
    /// Extended TTL (e.g. Anthropic "ephemeral" breakpoints).
    Extended,
}

// ---------------------------------------------------------------------------
// StreamOptions
// ---------------------------------------------------------------------------

/// Per-request options forwarded to the provider's `stream` call.
#[derive(Debug, Clone, Default)]
pub struct StreamOptions {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub api_key: Option<String>,
    pub cache_retention: Option<CacheRetention>,
    pub headers: Vec<(String, String)>,
    /// Enable extended thinking / reasoning.
    pub thinking_enabled: Option<bool>,
    /// Budget tokens for thinking (Anthropic budget-based, Google).
    pub thinking_budget_tokens: Option<u32>,
    /// Reasoning effort level (maps to provider-specific values).
    pub reasoning: Option<super::types::ReasoningLevel>,
    /// Session ID for prompt caching (OpenAI Responses API).
    pub session_id: Option<String>,
}

// ---------------------------------------------------------------------------
// ApiProvider trait
// ---------------------------------------------------------------------------

/// A pluggable LLM API backend.
///
/// Each provider registers itself under a unique API identifier (e.g.
/// `"openai-completions"`, `"anthropic-messages"`) and implements streaming
/// completion.
#[async_trait]
pub trait ApiProvider: Send + Sync {
    /// The API identifier this provider handles (e.g. `"openai-completions"`).
    fn api(&self) -> &str;

    /// Stream a completion response as a series of events.
    async fn stream(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
        options: &StreamOptions,
    ) -> Vec<AssistantMessageEvent>;
}

// ---------------------------------------------------------------------------
// Global registry
// ---------------------------------------------------------------------------

static REGISTRY: LazyLock<RwLock<HashMap<String, Arc<dyn ApiProvider>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Register a provider, keyed by its `api()` identifier.
///
/// If a provider with the same API identifier already exists it is replaced.
pub fn register_provider(provider: Arc<dyn ApiProvider>) {
    let api = provider.api().to_string();
    REGISTRY.write().unwrap().insert(api, provider);
}

/// Look up a provider by API identifier.
pub fn get_provider(api: &str) -> Option<Arc<dyn ApiProvider>> {
    REGISTRY.read().unwrap().get(api).cloned()
}

/// Remove all registered providers (useful for tests).
pub fn clear_providers() {
    REGISTRY.write().unwrap().clear();
}

/// Convenience: look up a provider or return an error string.
pub fn resolve_provider(api: &str) -> Result<Arc<dyn ApiProvider>, String> {
    get_provider(api).ok_or_else(|| format!("No provider registered for API: {api}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    struct MockProvider;

    #[async_trait]
    impl ApiProvider for MockProvider {
        fn api(&self) -> &str {
            "mock-api"
        }

        async fn stream(
            &self,
            _model: &Model,
            _ctx: &LlmContext,
            _tools: &[LlmTool],
            _opts: &StreamOptions,
        ) -> Vec<AssistantMessageEvent> {
            vec![AssistantMessageEvent::Done {
                stop_reason: crate::types::StopReason::Stop,
            }]
        }
    }

    #[test]
    #[serial]
    fn test_register_and_get() {
        clear_providers();
        register_provider(Arc::new(MockProvider));
        assert!(get_provider("mock-api").is_some());
        assert!(get_provider("unknown").is_none());
        clear_providers();
    }

    #[test]
    #[serial]
    fn test_resolve_provider_error() {
        clear_providers();
        let result = resolve_provider("nonexistent");
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.contains("nonexistent"));
    }
}
