// Concrete API provider implementations.

pub mod anthropic;
pub mod azure_openai_responses;
pub mod bedrock;
pub mod github_copilot_headers;
pub mod google;
pub mod google_gemini_cli;
pub mod google_shared;
pub mod google_vertex;
pub mod mistral;
pub mod openai_codex_responses;
pub mod openai_completions;
pub mod openai_responses;
pub(crate) mod openai_responses_shared;
pub mod simple_options;

pub use anthropic::AnthropicProvider;
pub use azure_openai_responses::AzureOpenAiResponsesProvider;
pub use bedrock::BedrockProvider;
pub use google::GoogleProvider;
pub use google_gemini_cli::GoogleGeminiCliProvider;
pub use google_vertex::GoogleVertexProvider;
pub use mistral::MistralProvider;
pub use openai_codex_responses::OpenAiCodexResponsesProvider;
pub use openai_completions::OpenAiCompletionsProvider;
pub use openai_responses::OpenAiResponsesProvider;

#[cfg(test)]
mod provider_integration_tests;
