// Translated from pi-mono packages/agent/test/e2e.test.ts
//
// E2E tests for the Agent across multiple LLM providers.
// Tests requiring real API keys are marked #[ignore].
// Validation tests (no network) run unconditionally.

use agent_core::{
    agent::{Agent, AgentOptions},
    agent_loop::LlmProvider,
    types::{
        AgentMessage, AgentTool, AgentToolResult, AssistantMessage, Content, StopReason,
        ThinkingLevel, ToolExecutionMode, ToolResultMessage, UserMessage,
    },
};
use ai::{
    registry::{ApiProvider, StreamOptions},
    types::{
        AssistantMessageEvent, InputType, LlmContext, LlmTool, Model, ModelCost, StopReason as SR,
        Usage,
    },
};
use std::sync::Arc;
use std::time::Duration;

fn ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// =============================================================================
// Helpers / mock providers
// =============================================================================

fn make_model(id: &str) -> Model {
    Model {
        id: id.into(),
        name: id.into(),
        api: ai::types::api::OPENAI_COMPLETIONS.into(),
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
        compat: None,
    }
}

/// Provider that returns a fixed sequence of responses.
struct SequenceProvider {
    responses: std::sync::Mutex<std::collections::VecDeque<Vec<AssistantMessageEvent>>>,
}

impl SequenceProvider {
    fn new(responses: Vec<Vec<AssistantMessageEvent>>) -> Self {
        Self {
            responses: std::sync::Mutex::new(std::collections::VecDeque::from(responses)),
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for SequenceProvider {
    async fn complete(
        &self,
        _model: &Model,
        _ctx: &LlmContext,
        _tools: &[LlmTool],
    ) -> Vec<AssistantMessageEvent> {
        let mut q = self.responses.lock().unwrap();
        q.pop_front().unwrap_or_else(|| {
            vec![AssistantMessageEvent::Done {
                stop_reason: SR::Stop,
            }]
        })
    }
}

fn text_response(text: &str) -> Vec<AssistantMessageEvent> {
    vec![
        AssistantMessageEvent::TextDelta(text.into()),
        AssistantMessageEvent::Done {
            stop_reason: SR::Stop,
        },
    ]
}

fn opts_with_provider(provider: Arc<dyn LlmProvider>) -> AgentOptions {
    AgentOptions::new(
        make_model("test-model"),
        "You are a helpful assistant.",
        provider,
    )
}

// =============================================================================
// Agent.continue() validation tests (no network)
// Translated from: Agent.continue() / validation
// =============================================================================

/// Translated from: "should throw when no messages in context"
#[tokio::test]
async fn agent_continue_throws_when_no_messages() {
    let provider = Arc::new(SequenceProvider::new(vec![]));
    let mut agent = Agent::new(opts_with_provider(provider));

    let result = agent.continue_run().await;
    assert!(result.is_err(), "continue with no messages should fail");
    let err = result.unwrap_err();
    assert!(
        err.contains("No messages to continue from"),
        "unexpected error: {err}"
    );
}

/// Translated from: "should throw when last message is assistant"
#[tokio::test]
async fn agent_continue_throws_when_last_message_is_assistant() {
    let provider = Arc::new(SequenceProvider::new(vec![]));
    let mut agent = Agent::new(opts_with_provider(provider));

    agent.replace_messages(vec![AgentMessage::Assistant(AssistantMessage::from_text(
        "Hello",
    ))]);

    let result = agent.continue_run().await;
    assert!(result.is_err(), "continue with assistant tail should fail");
    let err = result.unwrap_err();
    assert!(
        err.contains("Cannot continue from message role: assistant"),
        "unexpected error: {err}"
    );
}

// =============================================================================
// Mock-based functional tests (no real API needed)
// Translated from the shared helper functions: basicPrompt, toolExecution, etc.
// =============================================================================

/// Translated from: basicPrompt helper
/// Verifies agent processes a text prompt and produces user + assistant messages.
#[tokio::test]
async fn mock_basic_prompt_produces_two_messages() {
    let provider = Arc::new(SequenceProvider::new(vec![text_response("4")]));
    let mut agent = Agent::new(opts_with_provider(provider));

    let result = agent
        .prompt_text("What is 2+2? Answer with just the number.")
        .await;
    assert!(result.is_ok());
    assert!(!agent.is_streaming());
    assert_eq!(agent.messages().len(), 2);
    assert!(matches!(agent.messages()[0], AgentMessage::User(_)));
    assert!(matches!(agent.messages()[1], AgentMessage::Assistant(_)));

    if let AgentMessage::Assistant(a) = &agent.messages()[1] {
        assert!(!a.content.is_empty());
    }
}

/// Translated from: stateUpdates helper — "should emit state updates during streaming"
#[tokio::test]
async fn mock_state_updates_emitted_during_prompt() {
    use std::sync::Mutex;
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let events_clone = Arc::clone(&events);

    let provider = Arc::new(SequenceProvider::new(vec![text_response("1 2 3 4 5")]));
    let mut agent = Agent::new(opts_with_provider(provider));

    agent.subscribe(move |e| {
        events_clone.lock().unwrap().push(
            format!("{:?}", e)
                .split('{')
                .next()
                .unwrap_or("")
                .trim()
                .to_string(),
        );
    });

    agent.prompt_text("Count from 1 to 5.").await.unwrap();

    let collected = events.lock().unwrap();
    let type_strings: Vec<&str> = collected.iter().map(|s| s.as_str()).collect();
    assert!(
        type_strings.iter().any(|s| s.contains("AgentStart")),
        "missing AgentStart; got: {:?}",
        type_strings
    );
    assert!(
        type_strings.iter().any(|s| s.contains("AgentEnd")),
        "missing AgentEnd; got: {:?}",
        type_strings
    );
    assert!(!agent.is_streaming());
    assert_eq!(agent.messages().len(), 2);
}

/// Translated from: multiTurnConversation helper
#[tokio::test]
async fn mock_multi_turn_conversation() {
    let provider = Arc::new(SequenceProvider::new(vec![
        text_response("Nice to meet you, Alice."),
        text_response("Your name is Alice."),
    ]));
    let mut agent = Agent::new(opts_with_provider(provider));

    agent.prompt_text("My name is Alice.").await.unwrap();
    assert_eq!(agent.messages().len(), 2);

    agent.prompt_text("What is my name?").await.unwrap();
    assert_eq!(agent.messages().len(), 4);

    if let AgentMessage::Assistant(a) = &agent.messages()[3] {
        let text = a
            .content
            .iter()
            .filter_map(|c| {
                if let Content::Text { text } = c {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<String>();
        assert!(
            text.to_lowercase().contains("alice"),
            "last response should mention Alice: {text}"
        );
    } else {
        panic!("last message should be assistant");
    }
}

/// Translated from: toolExecution helper (using mock provider)
#[tokio::test]
async fn mock_tool_execution_produces_tool_result() {
    struct CalcTool;

    #[async_trait::async_trait]
    impl AgentTool for CalcTool {
        fn name(&self) -> &str {
            "calculate"
        }
        fn label(&self) -> &str {
            "Calculate"
        }
        fn description(&self) -> &str {
            "Evaluates a math expression"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": { "expression": { "type": "string" } }
            })
        }
        async fn execute(
            &self,
            _id: &str,
            args: serde_json::Value,
            _signal: Option<tokio_util::sync::CancellationToken>,
            _on_update: Option<&agent_core::types::OnUpdateFn>,
        ) -> AgentToolResult {
            let expr = args["expression"].as_str().unwrap_or("").to_string();
            // Very simple eval: only handles "X * Y"
            let result = if let Some((a, b)) = expr.split_once(" * ") {
                let a: i64 = a.trim().parse().unwrap_or(0);
                let b: i64 = b.trim().parse().unwrap_or(0);
                (a * b).to_string()
            } else {
                "unknown".to_string()
            };
            AgentToolResult {
                content: vec![Content::Text {
                    text: format!("{expr} = {result}"),
                }],
                details: serde_json::Value::Null,
            }
        }
    }

    let tool_call_response = vec![
        AssistantMessageEvent::ToolCallStart {
            id: "calc-1".into(),
            name: "calculate".into(),
        },
        AssistantMessageEvent::ToolCallDelta {
            id: "calc-1".into(),
            arguments_delta: r#"{"expression":"123 * 456"}"#.into(),
        },
        AssistantMessageEvent::ToolCallEnd {
            id: "calc-1".into(),
        },
        AssistantMessageEvent::Done {
            stop_reason: SR::ToolUse,
        },
    ];
    let final_response = text_response("123 * 456 = 56088");

    let provider = Arc::new(SequenceProvider::new(vec![
        tool_call_response,
        final_response,
    ]));
    let mut opts = opts_with_provider(provider);
    opts.tools = vec![Arc::new(CalcTool) as Arc<dyn AgentTool>];
    opts.system_prompt =
        "You are a helpful assistant. Always use the calculator tool for math.".into();

    let mut agent = Agent::new(opts);
    agent
        .prompt_text("Calculate 123 * 456 using the calculator tool.")
        .await
        .unwrap();

    assert!(!agent.is_streaming());
    assert!(agent.messages().len() >= 3);

    let tool_result = agent
        .messages()
        .iter()
        .find(|m| matches!(m, AgentMessage::ToolResult(_)));
    assert!(tool_result.is_some(), "expected a tool result message");

    let expected = 123i64 * 456;
    if let Some(AgentMessage::ToolResult(tr)) = tool_result {
        let text: String = tr
            .content
            .iter()
            .filter_map(|c| {
                if let Content::Text { text } = c {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            text.contains(&expected.to_string()),
            "tool result should contain {expected}: {text}"
        );
    }

    let last = agent.messages().last().unwrap();
    assert!(matches!(last, AgentMessage::Assistant(_)));
}

// =============================================================================
// DashScope / Qwen provider helpers
// Mirrors pi-mono's provider-agnostic test helpers using DashScope as the backend.
// =============================================================================

/// Build a Model pointing at DashScope's OpenAI-compatible endpoint.
fn qwen_model() -> Model {
    Model {
        id: "qwen-plus".into(),
        name: "Qwen Plus (DashScope)".into(),
        api: "openai-completions".into(),
        provider: "dashscope".into(),
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
        api_key_env: "DASHSCOPE_API_KEY".into(),
        reasoning: false,
        input: vec![InputType::Text],
        max_tokens: 4096,
        context_window: 32768,
        cost: ModelCost {
            input_per_million: 0.4,
            output_per_million: 1.2,
            cache_read_per_million: 0.0,
            cache_write_per_million: 0.0,
        },
        headers: vec![],
        compat: None,
    }
}

/// LlmProvider wrapper around OpenAiCompletionsProvider for use in agent-core tests.
struct DashScopeProvider(ai::providers::OpenAiCompletionsProvider);

impl DashScopeProvider {
    fn new() -> Self {
        Self(ai::providers::OpenAiCompletionsProvider::new())
    }
}

#[async_trait::async_trait]
impl LlmProvider for DashScopeProvider {
    async fn complete(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
    ) -> Vec<AssistantMessageEvent> {
        self.0
            .stream(model, context, tools, &StreamOptions::default())
            .await
    }
}

/// CalcTool for tool-execution e2e tests.
struct CalcToolReal;

#[async_trait::async_trait]
impl AgentTool for CalcToolReal {
    fn name(&self) -> &str {
        "calculate"
    }
    fn label(&self) -> &str {
        "Calculator"
    }
    fn description(&self) -> &str {
        "Evaluate mathematical expressions"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "The mathematical expression to evaluate"
                }
            },
            "required": ["expression"]
        })
    }
    async fn execute(
        &self,
        _id: &str,
        args: serde_json::Value,
        _signal: Option<tokio_util::sync::CancellationToken>,
        _on_update: Option<&agent_core::types::OnUpdateFn>,
    ) -> AgentToolResult {
        let expr = args["expression"].as_str().unwrap_or("").to_string();
        // Simple expression evaluator covering basic arithmetic
        let result = eval_expr(&expr);
        AgentToolResult {
            content: vec![Content::Text {
                text: format!("{expr} = {result}"),
            }],
            details: serde_json::Value::Null,
        }
    }
}

fn eval_expr(expr: &str) -> String {
    // Handles arithmetic expressions with +, -, *, /
    // Strips parentheses and whitespace, then tries each operator.
    let expr = expr.trim().trim_matches(|c| c == '(' || c == ')');
    let expr = expr.trim();

    // Try each operator (order: +/- before */ for splitting priority)
    let operators = ['+', '-', '*', '/'];
    for &op in &operators {
        if let Some(pos) = expr.rfind(op) {
            if pos > 0 {
                let lhs = expr[..pos]
                    .trim()
                    .trim_matches(|c| c == '(' || c == ')')
                    .trim();
                let rhs = expr[pos + 1..]
                    .trim()
                    .trim_matches(|c| c == '(' || c == ')')
                    .trim();
                let a: f64 = lhs.parse().unwrap_or(f64::NAN);
                let b: f64 = rhs.parse().unwrap_or(f64::NAN);
                if !a.is_nan() && !b.is_nan() {
                    let result = match op {
                        '+' => a + b,
                        '-' => a - b,
                        '*' => a * b,
                        '/' if b != 0.0 => a / b,
                        _ => continue,
                    };
                    if result.fract() == 0.0 {
                        return format!("{}", result as i64);
                    }
                    return format!("{result:.2}");
                }
            }
        }
    }
    // Fallback: return the expression as-is so the model can see what we got
    format!("Error: cannot evaluate '{expr}'")
}

// =============================================================================
// Qwen/DashScope E2E tests — real API calls
// Translated from pi-mono's per-provider describe blocks.
// Requires DASHSCOPE_API_KEY to be set (already in ~/.zshrc).
// Run with: cargo test -p agent-core --test e2e e2e_qwen -- --ignored
// =============================================================================

/// Translated from: basicPrompt — "should handle basic text prompt"
#[tokio::test]
#[ignore = "requires DASHSCOPE_API_KEY"]
async fn e2e_qwen_basic_prompt() {
    let provider = Arc::new(DashScopeProvider::new());
    let mut agent = Agent::new(AgentOptions::new(
        qwen_model(),
        "You are a helpful assistant. Keep your responses concise.",
        provider,
    ));

    agent
        .prompt_text("What is 2+2? Answer with just the number.")
        .await
        .unwrap();

    assert!(!agent.is_streaming());
    assert_eq!(
        agent.messages().len(),
        2,
        "should have user + assistant messages"
    );
    assert!(matches!(agent.messages()[0], AgentMessage::User(_)));
    assert!(matches!(agent.messages()[1], AgentMessage::Assistant(_)));

    if let AgentMessage::Assistant(a) = &agent.messages()[1] {
        assert!(!a.content.is_empty());
        let text: String = a
            .content
            .iter()
            .filter_map(|c| {
                if let Content::Text { text } = c {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            text.contains('4'),
            "response should contain '4', got: {text}"
        );
    } else {
        panic!("second message must be assistant");
    }
}

/// Translated from: toolExecution — "should execute tools correctly"
#[tokio::test]
#[ignore = "requires DASHSCOPE_API_KEY"]
async fn e2e_qwen_tool_execution() {
    let provider = Arc::new(DashScopeProvider::new());
    let mut opts = AgentOptions::new(
        qwen_model(),
        "You are a helpful assistant. Always use the calculator tool for math.",
        provider,
    );
    opts.tools = vec![Arc::new(CalcToolReal) as Arc<dyn AgentTool>];

    let mut agent = Agent::new(opts);

    // Safety: abort after 90s to prevent infinite tool-call loops.
    let abort_handle = agent.abort_handle();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(90)).await;
        abort_handle.cancel();
    });

    agent
        .prompt_text("Calculate 123 * 456 using the calculator tool.")
        .await
        .ok();

    assert!(!agent.is_streaming());
    assert!(
        agent.messages().len() >= 3,
        "should have at least user + assistant + tool result"
    );

    let tool_result = agent
        .messages()
        .iter()
        .find(|m| matches!(m, AgentMessage::ToolResult(_)));
    assert!(tool_result.is_some(), "should have a tool result message");

    let expected = 123i64 * 456;
    if let Some(AgentMessage::ToolResult(tr)) = tool_result {
        let text: String = tr
            .content
            .iter()
            .filter_map(|c| {
                if let Content::Text { text } = c {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            text.contains(&expected.to_string())
                || text.contains("56,088")
                || text.contains("56088"),
            "tool result should contain {expected}: {text}"
        );
    }

    let final_msg = agent.messages().last().unwrap();
    // Final message is either assistant (normal completion) or tool result (aborted mid-run)
    let is_terminal = matches!(
        final_msg,
        AgentMessage::Assistant(_) | AgentMessage::ToolResult(_)
    );
    assert!(
        is_terminal,
        "final message should be assistant or tool result"
    );
    if let AgentMessage::Assistant(a) = final_msg {
        let text: String = a
            .content
            .iter()
            .filter_map(|c| {
                if let Content::Text { text } = c {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            text.contains(&expected.to_string())
                || text.contains("56,088")
                || text.contains("56088"),
            "final message should reference the result: {text}"
        );
    }
}

/// Translated from: abortExecution — "should handle abort during execution"
///
/// Uses abort_handle() to pre-allocate the CancellationToken before the prompt
/// starts, then cancels it from a spawned task after 100ms.
#[tokio::test]
#[ignore = "requires DASHSCOPE_API_KEY"]
async fn e2e_qwen_abort_execution() {
    let provider = Arc::new(DashScopeProvider::new());
    let mut opts = AgentOptions::new(qwen_model(), "You are a helpful assistant.", provider);
    opts.tools = vec![Arc::new(CalcToolReal) as Arc<dyn AgentTool>];

    let mut agent = Agent::new(opts);

    // Pre-allocate the token so we can cancel it while prompt is running.
    let abort_token = agent.abort_handle();

    // Spawn the abort after 100 ms.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        abort_token.cancel();
    });

    agent
        .prompt_text("Calculate 100 * 200, then 300 * 400, then sum the results.")
        .await
        .ok(); // prompt may succeed or fail depending on abort timing

    // The prompt may succeed (model faster than 100ms) or abort fires mid-run.
    // Either way the agent must not be in a streaming state after the call returns.
    assert!(!agent.is_streaming());
    assert!(
        agent.messages().len() >= 2,
        "should have at least user + some message; got: {}",
        agent.messages().len()
    );

    // The last message may be:
    //   - AssistantMessage (abort fired before LLM started, or model finished fast)
    //   - ToolResult (abort fired between tool execution and the next LLM call)
    // Both are valid outcomes — the key invariant is that we're not streaming.
    let last = agent.messages().last().unwrap();
    let is_terminal = matches!(
        last,
        AgentMessage::Assistant(_) | AgentMessage::ToolResult(_)
    );
    assert!(
        is_terminal,
        "last message should be assistant or tool result"
    );

    // If the final message is an aborted assistant, check error linkage.
    if let AgentMessage::Assistant(a) = last {
        if a.stop_reason == StopReason::Aborted {
            assert!(
                a.error_message.is_some(),
                "aborted assistant should have error_message set"
            );
            assert_eq!(agent.error(), a.error_message.as_deref());
        }
    }
}

/// Translated from: stateUpdates — "should emit state updates during streaming"
#[tokio::test]
#[ignore = "requires DASHSCOPE_API_KEY"]
async fn e2e_qwen_state_updates() {
    use std::sync::Mutex;
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let events_clone = Arc::clone(&events);

    let provider = Arc::new(DashScopeProvider::new());
    let mut agent = Agent::new(AgentOptions::new(
        qwen_model(),
        "You are a helpful assistant.",
        provider,
    ));

    agent.subscribe(move |e| {
        let tag = format!("{:?}", e);
        let type_name = tag.split('{').next().unwrap_or("").trim().to_string();
        events_clone.lock().unwrap().push(type_name);
    });

    agent.prompt_text("Count from 1 to 5.").await.unwrap();

    let collected = events.lock().unwrap();
    let has_agent_start = collected.iter().any(|s| s.contains("AgentStart"));
    let has_agent_end = collected.iter().any(|s| s.contains("AgentEnd"));
    let has_message_start = collected.iter().any(|s| s.contains("MessageStart"));
    let has_message_end = collected.iter().any(|s| s.contains("MessageEnd"));
    let has_message_update = collected.iter().any(|s| s.contains("MessageUpdate"));

    assert!(
        has_agent_start,
        "should receive AgentStart; got: {collected:?}"
    );
    assert!(has_agent_end, "should receive AgentEnd; got: {collected:?}");
    assert!(
        has_message_start,
        "should receive MessageStart; got: {collected:?}"
    );
    assert!(
        has_message_end,
        "should receive MessageEnd; got: {collected:?}"
    );
    assert!(
        has_message_update,
        "should receive MessageUpdate; got: {collected:?}"
    );

    assert!(!agent.is_streaming());
    assert_eq!(agent.messages().len(), 2, "should have user + assistant");
}

/// Translated from: multiTurnConversation — "should maintain context across multiple turns"
#[tokio::test]
#[ignore = "requires DASHSCOPE_API_KEY"]
async fn e2e_qwen_multi_turn() {
    let provider = Arc::new(DashScopeProvider::new());
    let mut agent = Agent::new(AgentOptions::new(
        qwen_model(),
        "You are a helpful assistant.",
        provider,
    ));

    agent.prompt_text("My name is Alice.").await.unwrap();
    assert_eq!(agent.messages().len(), 2);

    agent.prompt_text("What is my name?").await.unwrap();
    assert_eq!(agent.messages().len(), 4);

    if let AgentMessage::Assistant(a) = &agent.messages()[3] {
        let text: String = a
            .content
            .iter()
            .filter_map(|c| {
                if let Content::Text { text } = c {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            text.to_lowercase().contains("alice"),
            "last response should mention Alice: {text}"
        );
    } else {
        panic!("fourth message should be assistant");
    }
}

// =============================================================================
// Agent.continue() / "continue from user message" and "continue from tool result"
// These require real OPENAI_API_KEY — marked #[ignore]
// =============================================================================

/// Translated from: "should continue and get response when last message is user"
#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn e2e_continue_from_user_message() {
    // This test requires a real OpenAI provider; skipped without API key.
    unimplemented!(
        "Implement when real OpenAI provider is wired into agent-core integration tests"
    );
}

/// Translated from: "should continue and process tool results"
#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn e2e_continue_from_tool_result() {
    unimplemented!(
        "Implement when real OpenAI provider is wired into agent-core integration tests"
    );
}

// =============================================================================
// Provider-specific E2E tests (all require real API keys)
// Translated from: Google / OpenAI / Anthropic / xAI / Groq / zAI / Bedrock describe blocks
// =============================================================================

/// Translated from: Google Provider "should handle basic text prompt"
#[tokio::test]
#[ignore = "requires GEMINI_API_KEY"]
async fn e2e_google_basic_prompt() {
    unimplemented!("requires real Google Gemini provider")
}

/// Translated from: Google Provider "should execute tools correctly"
#[tokio::test]
#[ignore = "requires GEMINI_API_KEY"]
async fn e2e_google_tool_execution() {
    unimplemented!("requires real Google Gemini provider")
}

/// Translated from: Google Provider "should handle abort during execution"
#[tokio::test]
#[ignore = "requires GEMINI_API_KEY"]
async fn e2e_google_abort_execution() {
    unimplemented!("requires real Google Gemini provider")
}

/// Translated from: Google Provider "should emit state updates during streaming"
#[tokio::test]
#[ignore = "requires GEMINI_API_KEY"]
async fn e2e_google_state_updates() {
    unimplemented!("requires real Google Gemini provider")
}

/// Translated from: Google Provider "should maintain context across multiple turns"
#[tokio::test]
#[ignore = "requires GEMINI_API_KEY"]
async fn e2e_google_multi_turn() {
    unimplemented!("requires real Google Gemini provider")
}

/// Translated from: OpenAI Provider "should handle basic text prompt"
#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn e2e_openai_basic_prompt() {
    unimplemented!("requires real OpenAI provider")
}

/// Translated from: OpenAI Provider "should execute tools correctly"
#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn e2e_openai_tool_execution() {
    unimplemented!("requires real OpenAI provider")
}

/// Translated from: OpenAI Provider "should handle abort during execution"
#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn e2e_openai_abort_execution() {
    unimplemented!("requires real OpenAI provider")
}

/// Translated from: OpenAI Provider "should emit state updates during streaming"
#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn e2e_openai_state_updates() {
    unimplemented!("requires real OpenAI provider")
}

/// Translated from: OpenAI Provider "should maintain context across multiple turns"
#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn e2e_openai_multi_turn() {
    unimplemented!("requires real OpenAI provider")
}

/// Translated from: Anthropic Provider "should handle basic text prompt"
#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY"]
async fn e2e_anthropic_basic_prompt() {
    unimplemented!("requires real Anthropic provider")
}

/// Translated from: Anthropic Provider "should execute tools correctly"
#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY"]
async fn e2e_anthropic_tool_execution() {
    unimplemented!("requires real Anthropic provider")
}

/// Translated from: Anthropic Provider "should handle abort during execution"
#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY"]
async fn e2e_anthropic_abort_execution() {
    unimplemented!("requires real Anthropic provider")
}

/// Translated from: Anthropic Provider "should emit state updates during streaming"
#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY"]
async fn e2e_anthropic_state_updates() {
    unimplemented!("requires real Anthropic provider")
}

/// Translated from: Anthropic Provider "should maintain context across multiple turns"
#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY"]
async fn e2e_anthropic_multi_turn() {
    unimplemented!("requires real Anthropic provider")
}

/// Translated from: xAI Provider "should handle basic text prompt"
#[tokio::test]
#[ignore = "requires XAI_API_KEY"]
async fn e2e_xai_basic_prompt() {
    unimplemented!("requires real xAI provider")
}

/// Translated from: xAI Provider "should execute tools correctly"
#[tokio::test]
#[ignore = "requires XAI_API_KEY"]
async fn e2e_xai_tool_execution() {
    unimplemented!("requires real xAI provider")
}

/// Translated from: xAI Provider "should handle abort during execution"
#[tokio::test]
#[ignore = "requires XAI_API_KEY"]
async fn e2e_xai_abort_execution() {
    unimplemented!("requires real xAI provider")
}

/// Translated from: xAI Provider "should emit state updates during streaming"
#[tokio::test]
#[ignore = "requires XAI_API_KEY"]
async fn e2e_xai_state_updates() {
    unimplemented!("requires real xAI provider")
}

/// Translated from: xAI Provider "should maintain context across multiple turns"
#[tokio::test]
#[ignore = "requires XAI_API_KEY"]
async fn e2e_xai_multi_turn() {
    unimplemented!("requires real xAI provider")
}

/// Translated from: Groq Provider "should handle basic text prompt"
#[tokio::test]
#[ignore = "requires GROQ_API_KEY"]
async fn e2e_groq_basic_prompt() {
    unimplemented!("requires real Groq provider")
}

/// Translated from: Groq Provider "should execute tools correctly"
#[tokio::test]
#[ignore = "requires GROQ_API_KEY"]
async fn e2e_groq_tool_execution() {
    unimplemented!("requires real Groq provider")
}

/// Translated from: Groq Provider "should handle abort during execution"
#[tokio::test]
#[ignore = "requires GROQ_API_KEY"]
async fn e2e_groq_abort_execution() {
    unimplemented!("requires real Groq provider")
}

/// Translated from: Groq Provider "should emit state updates during streaming"
#[tokio::test]
#[ignore = "requires GROQ_API_KEY"]
async fn e2e_groq_state_updates() {
    unimplemented!("requires real Groq provider")
}

/// Translated from: Groq Provider "should maintain context across multiple turns"
#[tokio::test]
#[ignore = "requires GROQ_API_KEY"]
async fn e2e_groq_multi_turn() {
    unimplemented!("requires real Groq provider")
}

/// Translated from: zAI Provider "should handle basic text prompt"
#[tokio::test]
#[ignore = "requires ZAI_API_KEY"]
async fn e2e_zai_basic_prompt() {
    unimplemented!("requires real zAI provider")
}

/// Translated from: zAI Provider "should execute tools correctly"
#[tokio::test]
#[ignore = "requires ZAI_API_KEY"]
async fn e2e_zai_tool_execution() {
    unimplemented!("requires real zAI provider")
}

/// Translated from: zAI Provider "should handle abort during execution"
#[tokio::test]
#[ignore = "requires ZAI_API_KEY"]
async fn e2e_zai_abort_execution() {
    unimplemented!("requires real zAI provider")
}

/// Translated from: zAI Provider "should emit state updates during streaming"
#[tokio::test]
#[ignore = "requires ZAI_API_KEY"]
async fn e2e_zai_state_updates() {
    unimplemented!("requires real zAI provider")
}

/// Translated from: zAI Provider "should maintain context across multiple turns"
#[tokio::test]
#[ignore = "requires ZAI_API_KEY"]
async fn e2e_zai_multi_turn() {
    unimplemented!("requires real zAI provider")
}

/// Translated from: Amazon Bedrock Provider "should handle basic text prompt"
#[tokio::test]
#[ignore = "requires AWS credentials"]
async fn e2e_bedrock_basic_prompt() {
    unimplemented!("requires real Bedrock provider")
}

/// Translated from: Amazon Bedrock Provider "should execute tools correctly"
#[tokio::test]
#[ignore = "requires AWS credentials"]
async fn e2e_bedrock_tool_execution() {
    unimplemented!("requires real Bedrock provider")
}

/// Translated from: Amazon Bedrock Provider "should handle abort during execution"
#[tokio::test]
#[ignore = "requires AWS credentials"]
async fn e2e_bedrock_abort_execution() {
    unimplemented!("requires real Bedrock provider")
}

/// Translated from: Amazon Bedrock Provider "should emit state updates during streaming"
#[tokio::test]
#[ignore = "requires AWS credentials"]
async fn e2e_bedrock_state_updates() {
    unimplemented!("requires real Bedrock provider")
}

/// Translated from: Amazon Bedrock Provider "should maintain context across multiple turns"
#[tokio::test]
#[ignore = "requires AWS credentials"]
async fn e2e_bedrock_multi_turn() {
    unimplemented!("requires real Bedrock provider")
}

/// Translated from: Agent.continue() / "continue from user message" (validation only)
#[tokio::test]
async fn agent_continue_from_user_message_state_is_valid() {
    let provider = Arc::new(SequenceProvider::new(vec![text_response("HELLO WORLD")]));
    let mut agent = Agent::new(opts_with_provider(provider));

    let user_msg = AgentMessage::User(UserMessage::from_text("Say exactly: HELLO WORLD"));
    agent.replace_messages(vec![user_msg]);

    agent.continue_run().await.unwrap();

    assert!(!agent.is_streaming());
    assert_eq!(agent.messages().len(), 2);
    assert!(matches!(agent.messages()[0], AgentMessage::User(_)));
    assert!(matches!(agent.messages()[1], AgentMessage::Assistant(_)));
}

/// Translated from: Agent.continue() / "continue from tool result"
#[tokio::test]
async fn agent_continue_from_tool_result_state_is_valid() {
    let provider = Arc::new(SequenceProvider::new(vec![text_response("5 + 3 = 8")]));
    let mut agent = Agent::new(opts_with_provider(provider));

    let user_msg = AgentMessage::User(UserMessage::from_text("What is 5 + 3?"));
    let assistant_msg = AgentMessage::Assistant(AssistantMessage {
        content: vec![
            Content::Text {
                text: "Let me calculate that.".into(),
            },
            Content::ToolCall {
                id: "calc-1".into(),
                name: "calculate".into(),
                arguments: serde_json::json!({"expression": "5 + 3"}),
            },
        ],
        provider: "test".into(),
        model: "test-model".into(),
        usage: Usage::default(),
        stop_reason: StopReason::ToolUse,
        error_message: None,
        timestamp: ts(),
    });
    let tool_result = AgentMessage::ToolResult(ToolResultMessage {
        tool_call_id: "calc-1".into(),
        tool_name: "calculate".into(),
        content: vec![Content::Text {
            text: "5 + 3 = 8".into(),
        }],
        details: None,
        is_error: false,
        timestamp: ts(),
    });

    agent.replace_messages(vec![user_msg, assistant_msg, tool_result]);

    agent.continue_run().await.unwrap();

    assert!(!agent.is_streaming());
    assert!(agent.messages().len() >= 4);

    let last = agent.messages().last().unwrap();
    assert!(matches!(last, AgentMessage::Assistant(_)));
}
