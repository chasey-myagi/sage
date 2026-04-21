//! Unified LLM API — mirrors pi-mono's `packages/ai/src/index.ts`.
//!
//! Provides a single streaming interface over multiple LLM providers:
//! Anthropic, OpenAI (Completions & Responses), Google (Gemini & Vertex),
//! Azure OpenAI, and AWS Bedrock, plus any OpenAI-compatible endpoint.

pub mod bedrock_provider;
pub mod cli;
pub mod keys;
pub mod model_pricing;
pub mod models;

pub mod openai_compat;
pub mod provider_errors;
pub mod provider_specs;
pub mod providers;
pub mod registry;
pub mod stream;
#[cfg(test)]
pub mod test_helpers;
pub mod types;
pub mod utils;

// NOTE: `transform.rs` is intentionally NOT declared as a module in the
// standalone `ai` crate. It references `AgentMessage` / `Content` types
// that live in `sage-runtime::types`, not in this crate. The file exists
// as a copy of the `sage-runtime::llm::transform` module for reference
// during the extraction work and should be deleted once extraction is
// complete. Declaring it would break compilation.

use std::sync::Arc;
pub use types::*;

/// Trait for LLM providers.
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
    ) -> Vec<AssistantMessageEvent>;
}

#[async_trait::async_trait]
impl<T: ?Sized + LlmProvider> LlmProvider for Arc<T> {
    async fn complete(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
    ) -> Vec<AssistantMessageEvent> {
        (**self).complete(model, context, tools).await
    }
}

/// Populate a [`registry::ApiProviderRegistry`] with all built-in providers.
pub fn register_builtin_into(reg: &registry::ApiProviderRegistry) {
    use providers::*;
    reg.register(Arc::new(AnthropicProvider::new()));
    reg.register(Arc::new(OpenAiCompletionsProvider::new()));
    reg.register(Arc::new(OpenAiResponsesProvider::new()));
    reg.register(Arc::new(GoogleProvider::new()));
    reg.register(Arc::new(AzureOpenAiResponsesProvider::new()));
    reg.register(Arc::new(BedrockProvider::new()));
    reg.register(Arc::new(GoogleVertexProvider::new()));
}

/// Register all built-in providers into the process-global registry.
///
/// Transitional shim for existing integration-test setup that still uses the
/// process-global registry.  New code should prefer `register_builtin_into`
/// with an instance-level [`registry::ApiProviderRegistry`] for isolation.
#[doc(hidden)]
pub fn register_builtin_providers() {
    use providers::*;
    registry::register_provider(Arc::new(AnthropicProvider::new()));
    registry::register_provider(Arc::new(OpenAiCompletionsProvider::new()));
    registry::register_provider(Arc::new(OpenAiResponsesProvider::new()));
    registry::register_provider(Arc::new(GoogleProvider::new()));
    registry::register_provider(Arc::new(AzureOpenAiResponsesProvider::new()));
    registry::register_provider(Arc::new(BedrockProvider::new()));
    registry::register_provider(Arc::new(GoogleVertexProvider::new()));
}

/// Stream a completion using the model's registered provider.
pub async fn stream(
    model: &Model,
    context: &LlmContext,
    tools: &[LlmTool],
    options: &registry::StreamOptions,
) -> Result<Vec<AssistantMessageEvent>, String> {
    let provider = registry::resolve_provider(&model.api)?;
    Ok(provider.stream(model, context, tools, options).await)
}

/// Collect a full completion (non-streaming convenience wrapper).
pub async fn complete(
    model: &Model,
    context: &LlmContext,
    tools: &[LlmTool],
    options: &registry::StreamOptions,
) -> Result<Vec<AssistantMessageEvent>, String> {
    stream(model, context, tools, options).await
}
