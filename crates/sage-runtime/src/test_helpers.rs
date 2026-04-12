// Shared test helpers — reusable Model/LlmContext constructors for tests.
// Only compiled in test builds.

use crate::llm::types::*;

/// Standard test model with sensible defaults.
pub fn test_model() -> Model {
    Model {
        id: "test-model".into(),
        name: "Test Model".into(),
        api: api::OPENAI_COMPLETIONS.into(),
        provider: "test".into(),
        base_url: "http://localhost".into(),
        api_key_env: "TEST_KEY".into(),
        reasoning: false,
        input: vec![InputType::Text],
        max_tokens: 4096,
        context_window: 32768,
        cost: ModelCost {
            input_per_million: 0.0,
            output_per_million: 0.0,
            cache_read_per_million: 0.0,
            cache_write_per_million: 0.0,
        },
        headers: vec![],
        compat: Some(ProviderCompat::default()),
    }
}

/// Standard test LLM context with empty messages.
pub fn test_context() -> LlmContext {
    LlmContext {
        messages: vec![],
        system_prompt: String::new(),
        max_tokens: 4096,
        temperature: None,
    }
}
