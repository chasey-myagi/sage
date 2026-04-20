// Shared test helpers for agent-core tests.
// Mirrors sage-runtime's test_helpers.rs, adapted for agent-core's LlmProvider trait.

use crate::agent_loop::LlmProvider;
use ai::types::{
    api, AssistantMessageEvent, InputType, LlmContext, LlmTool, Model, ModelCost, ProviderCompat,
    StopReason,
};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

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

/// Mock LLM provider — stateful, returns pre-configured response sequences.
pub struct StatefulProvider {
    responses: Mutex<VecDeque<Vec<AssistantMessageEvent>>>,
    call_count: AtomicUsize,
}

impl StatefulProvider {
    pub fn new(responses: Vec<Vec<AssistantMessageEvent>>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            call_count: AtomicUsize::new(0),
        }
    }

    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl LlmProvider for StatefulProvider {
    async fn complete(
        &self,
        _model: &Model,
        _context: &LlmContext,
        _tools: &[LlmTool],
    ) -> Vec<AssistantMessageEvent> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        let mut queue = self.responses.lock().unwrap();
        queue.pop_front().unwrap_or_else(|| {
            vec![AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            }]
        })
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
