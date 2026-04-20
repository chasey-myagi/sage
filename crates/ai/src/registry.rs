// Provider registry — matches pi-mono's stream.ts architecture.
// Thread-safe global registry of API providers keyed by their API identifier.

use super::types::*;
use async_trait::async_trait;
use futures::StreamExt as _;
use std::collections::HashMap;
use std::pin::Pin;
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
///
/// ## Implementing a provider
///
/// Override `stream_events()` to return a true lazy `Stream`. The blanket
/// `stream()` default collects all events for callers that still need a `Vec`.
/// Providers that have not yet migrated can keep their `stream()` override
/// and inherit a `stream_events()` default that wraps it.
#[async_trait]
pub trait ApiProvider: Send + Sync {
    /// The API identifier this provider handles (e.g. `"openai-completions"`).
    fn api(&self) -> &str;

    /// Stream a completion response as a lazy `Stream` of events.
    ///
    /// This is the primary method to implement. The default implementation
    /// falls back to `stream()` (collect-then-iterate), so providers that
    /// have not yet migrated continue to work without modification.
    fn stream_events<'a>(
        &'a self,
        model: &'a Model,
        context: &'a LlmContext,
        tools: &'a [LlmTool],
        options: &'a StreamOptions,
    ) -> Pin<Box<dyn futures::Stream<Item = AssistantMessageEvent> + Send + 'a>> {
        // Default: collect from the legacy `stream()` and replay as a stream.
        // Providers override this to avoid buffering the full response.
        Box::pin(futures::stream::once(async move {
            self.stream(model, context, tools, options).await
        })
        .flat_map(futures::stream::iter))
    }

    /// Collect a completion response as a `Vec` of events (buffered).
    ///
    /// The default implementation drives `stream_events()` to completion.
    /// Providers that implement `stream_events()` get this for free; providers
    /// that still override `stream()` directly keep their existing behaviour.
    async fn stream(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
        options: &StreamOptions,
    ) -> Vec<AssistantMessageEvent> {
        self.stream_events(model, context, tools, options)
            .collect::<Vec<_>>()
            .await
    }
}

// ---------------------------------------------------------------------------
// Instance registry (Sprint 11 task #55 — SageEngine-scoped registry)
// ---------------------------------------------------------------------------

/// A thread-safe registry of `ApiProvider` instances keyed by their
/// `api()` identifier.
///
/// Sprint 11 task #55: this is the **instance-level** alternative to the
/// global `static REGISTRY` below. Future wiring (v0.0.2+) will migrate
/// `SageEngine` to own a `Arc<ApiProviderRegistry>` field so multiple
/// engines can carry different provider sets (multi-tenant / test isolation).
///
/// The global `register_provider` / `get_provider` API still exists and
/// remains authoritative for backward compatibility; they delegate to
/// `GLOBAL_REGISTRY.register / .get`.
pub struct ApiProviderRegistry {
    providers: RwLock<HashMap<String, Arc<dyn ApiProvider>>>,
}

impl ApiProviderRegistry {
    /// New empty registry.
    pub fn new() -> Self {
        Self {
            providers: RwLock::new(HashMap::new()),
        }
    }

    /// Register a provider, keyed by its `api()` identifier. Replaces any
    /// existing entry with the same API.
    pub fn register(&self, provider: Arc<dyn ApiProvider>) {
        let api = provider.api().to_string();
        self.providers.write().unwrap().insert(api, provider);
    }

    /// Look up a provider by API identifier.
    pub fn get(&self, api: &str) -> Option<Arc<dyn ApiProvider>> {
        self.providers.read().unwrap().get(api).cloned()
    }

    /// Clear all registered providers.
    pub fn clear(&self) {
        self.providers.write().unwrap().clear();
    }

    /// Look up or return a formatted error string.
    pub fn resolve(&self, api: &str) -> Result<Arc<dyn ApiProvider>, String> {
        self.get(api).ok_or_else(|| format!("No provider registered for API: {api}"))
    }

    /// Number of providers currently registered (testing / observability).
    pub fn len(&self) -> usize {
        self.providers.read().unwrap().len()
    }

    /// Whether the registry has zero providers.
    pub fn is_empty(&self) -> bool {
        self.providers.read().unwrap().is_empty()
    }
}

impl Default for ApiProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Global registry (backward-compat shim delegating to GLOBAL_REGISTRY)
// ---------------------------------------------------------------------------

static GLOBAL_REGISTRY: LazyLock<ApiProviderRegistry> = LazyLock::new(ApiProviderRegistry::new);

/// Register a provider in the process-global registry.
///
/// Sprint 11 task #55: deprecation candidate. New code should prefer
/// `SageEngine`'s instance registry once the v0.0.2+ wiring lands so
/// multi-engine tests don't cross-pollute. Keeping the global shim for
/// one release to avoid breaking downstream crates.
pub fn register_provider(provider: Arc<dyn ApiProvider>) {
    GLOBAL_REGISTRY.register(provider);
}

/// Look up a provider in the process-global registry.
pub fn get_provider(api: &str) -> Option<Arc<dyn ApiProvider>> {
    GLOBAL_REGISTRY.get(api)
}

/// Remove all providers from the process-global registry.
pub fn clear_providers() {
    GLOBAL_REGISTRY.clear()
}

/// Resolve (or error) via the process-global registry.
pub fn resolve_provider(api: &str) -> Result<Arc<dyn ApiProvider>, String> {
    GLOBAL_REGISTRY.resolve(api)
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

    // ── Sprint 11 task #55: ApiProviderRegistry instance-level API ────────

    #[test]
    fn api_provider_registry_new_is_empty() {
        let r = ApiProviderRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn api_provider_registry_register_and_get_roundtrip() {
        let r = ApiProviderRegistry::new();
        r.register(Arc::new(MockProvider));
        assert_eq!(r.len(), 1);
        assert!(r.get("mock-api").is_some());
        assert!(r.get("not-there").is_none());
    }

    #[test]
    fn api_provider_registry_register_same_api_replaces() {
        let r = ApiProviderRegistry::new();
        r.register(Arc::new(MockProvider));
        r.register(Arc::new(MockProvider));
        assert_eq!(r.len(), 1, "same api id must replace, not append");
    }

    #[test]
    fn api_provider_registry_clear_removes_everything() {
        let r = ApiProviderRegistry::new();
        r.register(Arc::new(MockProvider));
        r.clear();
        assert!(r.is_empty());
    }

    #[test]
    fn api_provider_registry_resolve_returns_descriptive_err() {
        let r = ApiProviderRegistry::new();
        let err = r.resolve("missing-api").err().unwrap();
        assert!(err.contains("missing-api"));
    }

    #[test]
    fn two_independent_registries_do_not_share_state() {
        // The whole point of Sprint 11 task #55: instance isolation.
        let a = ApiProviderRegistry::new();
        let b = ApiProviderRegistry::new();
        a.register(Arc::new(MockProvider));
        assert!(a.get("mock-api").is_some());
        assert!(b.get("mock-api").is_none(), "registry B must not see A's provider");
    }
}
