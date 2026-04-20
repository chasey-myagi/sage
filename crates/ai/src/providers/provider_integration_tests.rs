// Integration tests — verify each HTTP-based LLM provider end-to-end:
// mockito mock server → provider.stream() → assert emitted events.
//
// Bedrock is excluded (uses AWS SDK, not direct HTTP).

use super::*;
use crate::registry::{ApiProvider, StreamOptions};
use crate::types::*;
use serial_test::serial;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Ensure reqwest bypasses system proxy for localhost connections.
/// macOS may have a system-level HTTP proxy that routes localhost traffic
/// through a proxy server, causing mockito mock mismatches.
fn ensure_no_proxy() {
    if std::env::var("NO_PROXY").is_err() {
        unsafe { std::env::set_var("NO_PROXY", "127.0.0.1,localhost") };
    }
}

fn make_model(api: &str, base_url: &str, model_id: &str, api_key_env: &str) -> Model {
    Model {
        id: model_id.into(),
        name: model_id.into(),
        api: api.into(),
        provider: api.into(),
        base_url: base_url.into(),
        api_key_env: api_key_env.into(),
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

fn make_context(user_msg: &str) -> LlmContext {
    LlmContext {
        messages: vec![LlmMessage::User {
            content: vec![LlmContent::Text(user_msg.into())],
        }],
        system_prompt: "You are helpful.".into(),
        max_tokens: 4096,
        temperature: None,
    }
}

fn make_options(api_key: &str) -> StreamOptions {
    StreamOptions {
        api_key: Some(api_key.into()),
        ..Default::default()
    }
}

/// Collect all TextDelta payloads into a single string.
fn collect_text(events: &[AssistantMessageEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            AssistantMessageEvent::TextDelta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

/// Extract all Usage events from the stream.
fn collect_usage(events: &[AssistantMessageEvent]) -> Vec<&crate::types::Usage> {
    events
        .iter()
        .filter_map(|e| match e {
            AssistantMessageEvent::Usage(u) => Some(u),
            _ => None,
        })
        .collect()
}

/// Find the first index of an event matching a given discriminant tag.
fn event_position(events: &[AssistantMessageEvent], tag: &str) -> Option<usize> {
    events.iter().position(|e| match (e, tag) {
        (AssistantMessageEvent::TextDelta(_), "TextDelta") => true,
        (AssistantMessageEvent::ThinkingDelta(_), "ThinkingDelta") => true,
        (AssistantMessageEvent::ThinkingBlockEnd { .. }, "ThinkingBlockEnd") => true,
        (AssistantMessageEvent::Usage(_), "Usage") => true,
        (AssistantMessageEvent::Done { .. }, "Done") => true,
        (AssistantMessageEvent::ToolCallStart { .. }, "ToolCallStart") => true,
        (AssistantMessageEvent::ToolCallDelta { .. }, "ToolCallDelta") => true,
        (AssistantMessageEvent::ToolCallEnd { .. }, "ToolCallEnd") => true,
        (AssistantMessageEvent::Error(_), "Error") => true,
        _ => false,
    })
}

/// Assert the events contain a Done event.
fn assert_has_done(events: &[AssistantMessageEvent]) {
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Done { .. })),
        "Expected Done event in stream, got: {events:?}"
    );
}

/// Assert no Error events.
fn assert_no_errors(events: &[AssistantMessageEvent]) {
    for e in events {
        if let AssistantMessageEvent::Error(msg) = e {
            panic!("Unexpected error event: {msg}");
        }
    }
}

// ===========================================================================
// 1. Anthropic Messages API — text
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_stream_text() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello world\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    assert!(collect_text(&events).contains("Hello world"));
    assert_has_done(&events);
}

// ===========================================================================
// 2. Anthropic — tool call
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_stream_tool_call() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_02\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_01\",\"name\":\"bash\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"command\\\":\\\"ls\\\"}\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":20}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("run ls"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);
    assert!(
        events.iter().any(
            |e| matches!(e, AssistantMessageEvent::ToolCallStart { name, .. } if name == "bash")
        ),
        "Expected ToolCallStart(bash)"
    );
    assert_has_done(&events);
}

// ===========================================================================
// 3. Anthropic — 401 error
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_error_401() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(401)
        .with_body("{\"error\":{\"message\":\"Invalid API key\"}}")
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("bad-key"))
        .await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Error(m) if m.contains("401"))),
        "Expected 401 error, got: {events:?}"
    );
}

// ===========================================================================
// 4. OpenAI Chat Completions — text
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_completions_stream_text() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
data: [DONE]\n\n",
        )
        .create_async()
        .await;

    let provider = OpenAiCompletionsProvider::new();
    let model = make_model(
        api::OPENAI_COMPLETIONS,
        &server.url(),
        "gpt-4",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    assert!(collect_text(&events).contains("Hello world"));
    assert_has_done(&events);
}

// ===========================================================================
// 5. OpenAI Completions — tool call
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_completions_stream_tool_call() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"id\":\"chatcmpl-2\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"call_01\",\"type\":\"function\",\"function\":{\"name\":\"bash\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chatcmpl-2\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"cmd\\\":\\\"ls\\\"}\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chatcmpl-2\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n\
data: [DONE]\n\n",
        )
        .create_async()
        .await;

    let provider = OpenAiCompletionsProvider::new();
    let model = make_model(
        api::OPENAI_COMPLETIONS,
        &server.url(),
        "gpt-4",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("run ls"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);
    assert!(
        events.iter().any(
            |e| matches!(e, AssistantMessageEvent::ToolCallStart { name, .. } if name == "bash")
        ),
        "Expected ToolCallStart(bash)"
    );
    assert_has_done(&events);
}

// ===========================================================================
// 6. OpenAI Completions — 401 error
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_completions_error_401() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/chat/completions")
        .with_status(401)
        .with_body("{\"error\":{\"message\":\"Invalid API key\"}}")
        .create_async()
        .await;

    let provider = OpenAiCompletionsProvider::new();
    let model = make_model(
        api::OPENAI_COMPLETIONS,
        &server.url(),
        "gpt-4",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("bad-key"))
        .await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Error(m) if m.contains("401"))),
        "Expected 401 error, got: {events:?}"
    );
}

// ===========================================================================
// 7. OpenAI Responses API — text
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_responses_stream_text() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/responses")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
event: response.output_item.added\n\
data: {\"item\":{\"type\":\"message\",\"id\":\"item_01\",\"index\":0,\"status\":\"in_progress\"}}\n\n\
event: response.output_text.delta\n\
data: {\"delta\":\"Hello world\",\"item_index\":0}\n\n\
event: response.output_item.done\n\
data: {\"item\":{\"type\":\"message\",\"id\":\"item_01\",\"index\":0,\"status\":\"completed\"}}\n\n\
event: response.completed\n\
data: {\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}\n\n",
        )
        .create_async()
        .await;

    let provider = OpenAiResponsesProvider::new();
    let model = make_model(
        api::OPENAI_RESPONSES,
        &server.url(),
        "gpt-4o",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    assert!(collect_text(&events).contains("Hello world"));
    assert_has_done(&events);
}

// ===========================================================================
// 8. Google Generative AI — text
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_google_stream_text() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", mockito::Matcher::Regex(
            r"/models/gemini-2\.0-flash:streamGenerateContent\?.*".into(),
        ))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Hello world\"}]},\"finishReason\":null,\"index\":0}]}\n\n\
data: {\"candidates\":[{\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":5,\"totalTokenCount\":15}}\n\n",
        )
        .create_async()
        .await;

    let provider = GoogleProvider::new();
    let model = make_model(
        api::GOOGLE_GENERATIVE_AI,
        &server.url(),
        "gemini-2.0-flash",
        "GOOGLE_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    assert!(collect_text(&events).contains("Hello world"));
    assert_has_done(&events);
}

// ===========================================================================
// 9. Azure OpenAI Responses — text
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_azure_openai_responses_stream_text() {
    // Clear Azure env vars so provider falls back to model.base_url
    unsafe {
        std::env::remove_var("AZURE_OPENAI_BASE_URL");
        std::env::remove_var("AZURE_OPENAI_RESOURCE_NAME");
        std::env::remove_var("AZURE_OPENAI_DEPLOYMENT_NAME_MAP");
        std::env::remove_var("AZURE_OPENAI_API_VERSION");
    }

    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", mockito::Matcher::Regex(r"/responses\?api-version=.*".into()))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
event: response.output_item.added\n\
data: {\"item\":{\"type\":\"message\",\"id\":\"item_01\",\"index\":0,\"status\":\"in_progress\"}}\n\n\
event: response.output_text.delta\n\
data: {\"delta\":\"Hello world\",\"item_index\":0}\n\n\
event: response.output_item.done\n\
data: {\"item\":{\"type\":\"message\",\"id\":\"item_01\",\"index\":0,\"status\":\"completed\"}}\n\n\
event: response.completed\n\
data: {\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}\n\n",
        )
        .create_async()
        .await;

    let provider = AzureOpenAiResponsesProvider::new();
    let model = make_model(
        api::AZURE_OPENAI_RESPONSES,
        &server.url(),
        "gpt-4o",
        "AZURE_OPENAI_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    assert!(collect_text(&events).contains("Hello world"));
    assert_has_done(&events);
}

// ===========================================================================
// 10. Google Vertex AI — text
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_google_vertex_stream_text() {
    unsafe {
        std::env::set_var("GOOGLE_CLOUD_PROJECT", "test-project");
        std::env::set_var("GOOGLE_CLOUD_LOCATION", "us-central1");
    }

    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", mockito::Matcher::Regex(
            r"/v1/projects/test-project/locations/us-central1/publishers/google/models/gemini-2\.0-flash:streamGenerateContent\?.*".into(),
        ))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Hello world\"}]},\"finishReason\":null,\"index\":0}]}\n\n\
data: {\"candidates\":[{\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":5,\"totalTokenCount\":15}}\n\n",
        )
        .create_async()
        .await;

    let provider = GoogleVertexProvider::new();
    let model = make_model(
        api::GOOGLE_VERTEX,
        &server.url(),
        "gemini-2.0-flash",
        "GOOGLE_CLOUD_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    assert!(collect_text(&events).contains("Hello world"));
    assert_has_done(&events);

    unsafe {
        std::env::remove_var("GOOGLE_CLOUD_PROJECT");
        std::env::remove_var("GOOGLE_CLOUD_LOCATION");
    }
}

// ===========================================================================
// 11. OpenAI Responses — tool call
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_responses_stream_tool_call() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/responses")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
event: response.output_item.added\n\
data: {\"item\":{\"type\":\"function_call\",\"id\":\"fc_01\",\"call_id\":\"call_01\",\"name\":\"bash\",\"index\":0,\"status\":\"in_progress\"}}\n\n\
event: response.function_call_arguments.delta\n\
data: {\"delta\":\"{\\\"command\\\":\\\"ls\\\"}\",\"item_id\":\"fc_01\",\"call_id\":\"call_01\"}\n\n\
event: response.output_item.done\n\
data: {\"item\":{\"type\":\"function_call\",\"id\":\"fc_01\",\"call_id\":\"call_01\",\"name\":\"bash\",\"index\":0,\"status\":\"completed\",\"arguments\":\"{\\\"command\\\":\\\"ls\\\"}\"}}\n\n\
event: response.completed\n\
data: {\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":15}}}\n\n",
        )
        .create_async()
        .await;

    let provider = OpenAiResponsesProvider::new();
    let model = make_model(
        api::OPENAI_RESPONSES,
        &server.url(),
        "gpt-4o",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("run ls"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);
    assert!(
        events.iter().any(
            |e| matches!(e, AssistantMessageEvent::ToolCallStart { name, .. } if name == "bash")
        ),
        "Expected ToolCallStart(bash)"
    );
    assert_has_done(&events);
}

// ===========================================================================
// 12. OpenAI Responses — error
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_responses_error_401() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/responses")
        .with_status(401)
        .with_body("{\"error\":{\"message\":\"Invalid API key\"}}")
        .create_async()
        .await;

    let provider = OpenAiResponsesProvider::new();
    let model = make_model(
        api::OPENAI_RESPONSES,
        &server.url(),
        "gpt-4o",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("bad-key"))
        .await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Error(m) if m.contains("401"))),
        "Expected 401 error, got: {events:?}"
    );
}

// ===========================================================================
// 13. Google — tool call
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_google_stream_tool_call() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", mockito::Matcher::Regex(
            r"/models/gemini-2\.0-flash:streamGenerateContent\?.*".into(),
        ))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"functionCall\":{\"name\":\"bash\",\"args\":{\"command\":\"ls\"}}}]},\"finishReason\":null,\"index\":0}]}\n\n\
data: {\"candidates\":[{\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":5,\"totalTokenCount\":15}}\n\n",
        )
        .create_async()
        .await;

    let provider = GoogleProvider::new();
    let model = make_model(
        api::GOOGLE_GENERATIVE_AI,
        &server.url(),
        "gemini-2.0-flash",
        "GOOGLE_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("run ls"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);
    assert!(
        events.iter().any(
            |e| matches!(e, AssistantMessageEvent::ToolCallStart { name, .. } if name == "bash")
        ),
        "Expected ToolCallStart(bash)"
    );
    assert_has_done(&events);
}

// ===========================================================================
// 14. Google — error
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_google_error_403() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock(
            "POST",
            mockito::Matcher::Regex(r"/models/gemini-2\.0-flash:streamGenerateContent\?.*".into()),
        )
        .with_status(403)
        .with_body("{\"error\":{\"message\":\"Forbidden\"}}")
        .create_async()
        .await;

    let provider = GoogleProvider::new();
    let model = make_model(
        api::GOOGLE_GENERATIVE_AI,
        &server.url(),
        "gemini-2.0-flash",
        "GOOGLE_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("bad-key"))
        .await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Error(m) if m.contains("403"))),
        "Expected 403 error, got: {events:?}"
    );
}

// ===========================================================================
// 15. Azure — tool call
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_azure_openai_responses_stream_tool_call() {
    unsafe {
        std::env::remove_var("AZURE_OPENAI_BASE_URL");
        std::env::remove_var("AZURE_OPENAI_RESOURCE_NAME");
        std::env::remove_var("AZURE_OPENAI_DEPLOYMENT_NAME_MAP");
        std::env::remove_var("AZURE_OPENAI_API_VERSION");
    }

    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", mockito::Matcher::Regex(r"/responses\?api-version=.*".into()))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
event: response.output_item.added\n\
data: {\"item\":{\"type\":\"function_call\",\"id\":\"fc_01\",\"call_id\":\"call_01\",\"name\":\"bash\",\"index\":0,\"status\":\"in_progress\"}}\n\n\
event: response.function_call_arguments.delta\n\
data: {\"delta\":\"{\\\"command\\\":\\\"ls\\\"}\",\"item_id\":\"fc_01\",\"call_id\":\"call_01\"}\n\n\
event: response.output_item.done\n\
data: {\"item\":{\"type\":\"function_call\",\"id\":\"fc_01\",\"call_id\":\"call_01\",\"name\":\"bash\",\"index\":0,\"status\":\"completed\",\"arguments\":\"{\\\"command\\\":\\\"ls\\\"}\"}}\n\n\
event: response.completed\n\
data: {\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":15}}}\n\n",
        )
        .create_async()
        .await;

    let provider = AzureOpenAiResponsesProvider::new();
    let model = make_model(
        api::AZURE_OPENAI_RESPONSES,
        &server.url(),
        "gpt-4o",
        "AZURE_OPENAI_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("run ls"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);
    assert!(
        events.iter().any(
            |e| matches!(e, AssistantMessageEvent::ToolCallStart { name, .. } if name == "bash")
        ),
        "Expected ToolCallStart(bash)"
    );
    assert_has_done(&events);
}

// ===========================================================================
// 16. Azure — error
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_azure_openai_responses_error_401() {
    unsafe {
        std::env::remove_var("AZURE_OPENAI_BASE_URL");
        std::env::remove_var("AZURE_OPENAI_RESOURCE_NAME");
        std::env::remove_var("AZURE_OPENAI_DEPLOYMENT_NAME_MAP");
        std::env::remove_var("AZURE_OPENAI_API_VERSION");
    }

    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock(
            "POST",
            mockito::Matcher::Regex(r"/responses\?api-version=.*".into()),
        )
        .with_status(401)
        .with_body("{\"error\":{\"message\":\"Invalid API key\"}}")
        .create_async()
        .await;

    let provider = AzureOpenAiResponsesProvider::new();
    let model = make_model(
        api::AZURE_OPENAI_RESPONSES,
        &server.url(),
        "gpt-4o",
        "AZURE_OPENAI_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("bad-key"))
        .await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Error(m) if m.contains("401"))),
        "Expected 401 error, got: {events:?}"
    );
}

// ===========================================================================
// 17. Vertex — error
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_google_vertex_error_403() {
    unsafe {
        std::env::set_var("GOOGLE_CLOUD_PROJECT", "test-project");
        std::env::set_var("GOOGLE_CLOUD_LOCATION", "us-central1");
    }

    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", mockito::Matcher::Regex(
            r"/v1/projects/test-project/locations/us-central1/publishers/google/models/gemini-2\.0-flash:streamGenerateContent\?.*".into(),
        ))
        .with_status(403)
        .with_body("{\"error\":{\"message\":\"Forbidden\"}}")
        .create_async()
        .await;

    let provider = GoogleVertexProvider::new();
    let model = make_model(
        api::GOOGLE_VERTEX,
        &server.url(),
        "gemini-2.0-flash",
        "GOOGLE_CLOUD_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("bad-key"))
        .await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Error(m) if m.contains("403"))),
        "Expected 403 error, got: {events:?}"
    );

    unsafe {
        std::env::remove_var("GOOGLE_CLOUD_PROJECT");
        std::env::remove_var("GOOGLE_CLOUD_LOCATION");
    }
}

// ===========================================================================
// 18. Anthropic — text + tool call mixed stream
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_stream_text_then_tool_call() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_03\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Let me check.\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_02\",\"name\":\"bash\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"command\\\":\\\"ls\\\"}\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":1}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":30}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("check files"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);
    // Should have both text and tool call
    assert!(collect_text(&events).contains("Let me check."));
    assert!(
        events.iter().any(
            |e| matches!(e, AssistantMessageEvent::ToolCallStart { name, .. } if name == "bash")
        ),
        "Expected ToolCallStart(bash)"
    );
    assert_has_done(&events);
}

// ===========================================================================
// 19. OpenAI Completions — stop_reason mapping
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_completions_stop_reason_length() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"id\":\"chatcmpl-3\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"truncated\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chatcmpl-3\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"length\"}]}\n\n\
data: [DONE]\n\n",
        )
        .create_async()
        .await;

    let provider = OpenAiCompletionsProvider::new();
    let model = make_model(
        api::OPENAI_COMPLETIONS,
        &server.url(),
        "gpt-4",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    // finish_reason: "length" should map to StopReason::Length
    assert!(
        events.iter().any(
            |e| matches!(e, AssistantMessageEvent::Done { stop_reason } if *stop_reason == crate::types::StopReason::Length)
        ),
        "Expected Done with StopReason::Length"
    );
}

// ===========================================================================
// 20. Anthropic — stop_reason mapping (end_turn → Stop)
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_stop_reason_end_turn() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_04\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    assert!(
        events.iter().any(
            |e| matches!(e, AssistantMessageEvent::Done { stop_reason } if *stop_reason == crate::types::StopReason::Stop)
        ),
        "Expected Done with StopReason::Stop for end_turn"
    );
}

// ===========================================================================
// 21. Anthropic — stop_reason mapping (max_tokens → Length)
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_stop_reason_max_tokens() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_05\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"truncated output\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\"},\"usage\":{\"output_tokens\":100}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    assert!(
        events.iter().any(
            |e| matches!(e, AssistantMessageEvent::Done { stop_reason } if *stop_reason == crate::types::StopReason::Length)
        ),
        "Expected Done with StopReason::Length for max_tokens"
    );
}

// ===========================================================================
// 22. Google — empty candidates (no text, just STOP)
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_google_stream_empty_response() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", mockito::Matcher::Regex(
            r"/models/gemini-2\.0-flash:streamGenerateContent\?.*".into(),
        ))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"candidates\":[{\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":0,\"totalTokenCount\":10}}\n\n",
        )
        .create_async()
        .await;

    let provider = GoogleProvider::new();
    let model = make_model(
        api::GOOGLE_GENERATIVE_AI,
        &server.url(),
        "gemini-2.0-flash",
        "GOOGLE_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    assert!(collect_text(&events).is_empty());
    assert_has_done(&events);
}

// ===========================================================================
// 23. OpenAI Completions — non-JSON error body
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_completions_error_html_body() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/chat/completions")
        .with_status(500)
        .with_body("<html><body>Internal Server Error</body></html>")
        .create_async()
        .await;

    let provider = OpenAiCompletionsProvider::new();
    let model = make_model(
        api::OPENAI_COMPLETIONS,
        &server.url(),
        "gpt-4",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Error(m) if m.contains("500"))),
        "Expected 500 error, got: {events:?}"
    );
}

// ===========================================================================
// 24. Anthropic — SSE error event in-stream
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_stream_sse_error_event() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_06\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n\
data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    // Should have the partial text
    assert!(collect_text(&events).contains("partial"));
    // Should also have an error event
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Error(..))),
        "Expected error event for overloaded_error"
    );
}

// ===========================================================================
// 25. OpenAI Completions — empty SSE stream (just [DONE])
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_completions_empty_stream() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body("data: [DONE]\n\n")
        .create_async()
        .await;

    let provider = OpenAiCompletionsProvider::new();
    let model = make_model(
        api::OPENAI_COMPLETIONS,
        &server.url(),
        "gpt-4",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    assert!(collect_text(&events).is_empty());
}

// ===========================================================================
// 26. Anthropic — empty API key returns error (no HTTP call)
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_missing_api_key() {
    ensure_no_proxy();
    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        "http://localhost:1",
        "claude-sonnet",
        "NONEXISTENT_API_KEY_ENV_VAR",
    );
    let opts = StreamOptions::default(); // no api_key
    let events = provider
        .stream(&model, &make_context("hi"), &[], &opts)
        .await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Error(..))),
        "Expected error for missing API key, got: {events:?}"
    );
}

// ===========================================================================
// 27. Provider registry — all 7 providers registered
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_all_providers_registered() {
    ensure_no_proxy();
    crate::register_builtin_providers();

    let expected_apis = [
        api::ANTHROPIC_MESSAGES,
        api::OPENAI_COMPLETIONS,
        api::OPENAI_RESPONSES,
        api::GOOGLE_GENERATIVE_AI,
        api::AZURE_OPENAI_RESPONSES,
        api::BEDROCK_CONVERSE_STREAM,
        api::GOOGLE_VERTEX,
    ];

    for api_id in &expected_apis {
        assert!(
            crate::registry::get_provider(api_id).is_some(),
            "Provider for '{api_id}' should be registered"
        );
    }
}

// ===========================================================================
// 28. Anthropic — Usage event verification
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_usage_event_tokens() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_u1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":25,\"cache_read_input_tokens\":5,\"cache_creation_input_tokens\":3}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":8}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    let usages = collect_usage(&events);
    // Anthropic emits two Usage events: message_start (input) + message_delta (output)
    assert!(
        usages.len() >= 2,
        "Expected at least 2 Usage events, got {}",
        usages.len()
    );

    // First usage: input tokens from message_start
    assert_eq!(usages[0].input, 25);
    assert_eq!(usages[0].cache_read, 5);
    assert_eq!(usages[0].cache_write, 3);
    assert_eq!(usages[0].output, 0);

    // Second usage: output tokens from message_delta
    assert_eq!(usages[1].output, 8);
    assert_eq!(usages[1].input, 0);
}

// ===========================================================================
// 29. OpenAI Completions — Usage event verification
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_completions_usage_event_tokens() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"id\":\"chatcmpl-u1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hi\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chatcmpl-u1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
data: {\"id\":\"chatcmpl-u1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[],\"usage\":{\"prompt_tokens\":15,\"completion_tokens\":3,\"total_tokens\":18}}\n\n\
data: [DONE]\n\n",
        )
        .create_async()
        .await;

    let provider = OpenAiCompletionsProvider::new();
    let model = make_model(
        api::OPENAI_COMPLETIONS,
        &server.url(),
        "gpt-4",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    let usages = collect_usage(&events);
    assert!(!usages.is_empty(), "Expected at least one Usage event");
    assert_eq!(usages[0].input, 15);
    assert_eq!(usages[0].output, 3);
    assert_eq!(usages[0].total_tokens, 18);
}

// ===========================================================================
// 30. OpenAI Responses — Usage event verification (with cached tokens)
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_responses_usage_event_tokens() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/responses")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
event: response.output_item.added\n\
data: {\"item\":{\"type\":\"message\",\"id\":\"item_u1\",\"index\":0,\"status\":\"in_progress\"}}\n\n\
event: response.output_text.delta\n\
data: {\"delta\":\"Hi\",\"item_index\":0}\n\n\
event: response.output_item.done\n\
data: {\"item\":{\"type\":\"message\",\"id\":\"item_u1\",\"index\":0,\"status\":\"completed\"}}\n\n\
event: response.completed\n\
data: {\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":30,\"output_tokens\":12,\"input_tokens_details\":{\"cached_tokens\":10}}}}\n\n",
        )
        .create_async()
        .await;

    let provider = OpenAiResponsesProvider::new();
    let model = make_model(
        api::OPENAI_RESPONSES,
        &server.url(),
        "gpt-4o",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    let usages = collect_usage(&events);
    assert!(
        !usages.is_empty(),
        "Expected Usage event from response.completed"
    );
    // input_tokens(30) - cached_tokens(10) = 20
    assert_eq!(usages[0].input, 20);
    assert_eq!(usages[0].output, 12);
    assert_eq!(usages[0].cache_read, 10);
    assert_eq!(usages[0].total_tokens, 42); // 30 + 12
}

// ===========================================================================
// 31. Google — Usage event verification
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_google_usage_event_tokens() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", mockito::Matcher::Regex(
            r"/models/gemini-2\.0-flash:streamGenerateContent\?.*".into(),
        ))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Hi\"}]},\"finishReason\":null,\"index\":0}]}\n\n\
data: {\"candidates\":[{\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":20,\"candidatesTokenCount\":8,\"totalTokenCount\":28}}\n\n",
        )
        .create_async()
        .await;

    let provider = GoogleProvider::new();
    let model = make_model(
        api::GOOGLE_GENERATIVE_AI,
        &server.url(),
        "gemini-2.0-flash",
        "GOOGLE_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    let usages = collect_usage(&events);
    assert!(
        !usages.is_empty(),
        "Expected Usage event from usageMetadata"
    );
    assert_eq!(usages[0].input, 20);
    assert_eq!(usages[0].output, 8);
    assert_eq!(usages[0].total_tokens, 28);
}

// ===========================================================================
// 32. Anthropic — ToolCall full lifecycle (Start → Delta → End)
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_tool_call_lifecycle() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_tc1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_lc1\",\"name\":\"read_file\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"/tmp\\\"}\" }}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":15}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("read /tmp"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);

    // Verify ToolCallStart
    assert!(
        events.iter().any(|e| matches!(
            e,
            AssistantMessageEvent::ToolCallStart { id, name }
            if id == "toolu_lc1" && name == "read_file"
        )),
        "Expected ToolCallStart with id=toolu_lc1, name=read_file"
    );

    // Verify ToolCallDelta with argument content
    assert!(
        events.iter().any(|e| matches!(
            e,
            AssistantMessageEvent::ToolCallDelta { id, arguments_delta }
            if id == "toolu_lc1" && arguments_delta.contains("/tmp")
        )),
        "Expected ToolCallDelta with /tmp in arguments"
    );

    // Verify ToolCallEnd
    assert!(
        events.iter().any(|e| matches!(
            e,
            AssistantMessageEvent::ToolCallEnd { id } if id == "toolu_lc1"
        )),
        "Expected ToolCallEnd with id=toolu_lc1"
    );

    // Verify ordering: Start before Delta before End
    let start_pos = event_position(&events, "ToolCallStart").unwrap();
    let delta_pos = event_position(&events, "ToolCallDelta").unwrap();
    let end_pos = event_position(&events, "ToolCallEnd").unwrap();
    assert!(
        start_pos < delta_pos,
        "ToolCallStart should precede ToolCallDelta"
    );
    assert!(
        delta_pos < end_pos,
        "ToolCallDelta should precede ToolCallEnd"
    );
}

// ===========================================================================
// 33. OpenAI Responses — ToolCall full lifecycle (Start → Delta → End)
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_responses_tool_call_lifecycle() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/responses")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
event: response.output_item.added\n\
data: {\"item\":{\"type\":\"function_call\",\"id\":\"fc_lc1\",\"call_id\":\"call_lc1\",\"name\":\"read_file\",\"index\":0,\"status\":\"in_progress\"}}\n\n\
event: response.function_call_arguments.delta\n\
data: {\"delta\":\"{\\\"path\\\":\\\"/tmp\\\"}\",\"item_id\":\"fc_lc1\",\"call_id\":\"call_lc1\"}\n\n\
event: response.output_item.done\n\
data: {\"item\":{\"type\":\"function_call\",\"id\":\"fc_lc1\",\"call_id\":\"call_lc1\",\"name\":\"read_file\",\"index\":0,\"status\":\"completed\",\"arguments\":\"{\\\"path\\\":\\\"/tmp\\\"}\"}}\n\n\
event: response.completed\n\
data: {\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":15}}}\n\n",
        )
        .create_async()
        .await;

    let provider = OpenAiResponsesProvider::new();
    let model = make_model(
        api::OPENAI_RESPONSES,
        &server.url(),
        "gpt-4o",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("read /tmp"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);

    // Verify full lifecycle: Start → Delta → End
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::ToolCallStart { name, .. } if name == "read_file"
        )),
        "Expected ToolCallStart(read_file)"
    );
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::ToolCallDelta { arguments_delta, .. }
            if arguments_delta.contains("/tmp")
        )),
        "Expected ToolCallDelta with /tmp"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::ToolCallEnd { .. })),
        "Expected ToolCallEnd"
    );

    // Ordering
    let start_pos = event_position(&events, "ToolCallStart").unwrap();
    let delta_pos = event_position(&events, "ToolCallDelta").unwrap();
    let end_pos = event_position(&events, "ToolCallEnd").unwrap();
    assert!(start_pos < delta_pos, "Start before Delta");
    assert!(delta_pos < end_pos, "Delta before End");
}

// ===========================================================================
// 34. Google — ToolCall lifecycle (atomic: Start + Delta + End in one SSE)
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_google_tool_call_lifecycle() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", mockito::Matcher::Regex(
            r"/models/gemini-2\.0-flash:streamGenerateContent\?.*".into(),
        ))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"functionCall\":{\"name\":\"read_file\",\"args\":{\"path\":\"/tmp\"}}}]},\"finishReason\":null,\"index\":0}]}\n\n\
data: {\"candidates\":[{\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":5,\"totalTokenCount\":15}}\n\n",
        )
        .create_async()
        .await;

    let provider = GoogleProvider::new();
    let model = make_model(
        api::GOOGLE_GENERATIVE_AI,
        &server.url(),
        "gemini-2.0-flash",
        "GOOGLE_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("read /tmp"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);

    // Google emits Start + Delta + End atomically from a single functionCall part
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::ToolCallStart { name, .. } if name == "read_file"
        )),
        "Expected ToolCallStart(read_file)"
    );
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::ToolCallDelta { arguments_delta, .. }
            if arguments_delta.contains("/tmp")
        )),
        "Expected ToolCallDelta with /tmp"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::ToolCallEnd { .. })),
        "Expected ToolCallEnd"
    );

    // All three should be in order even in atomic emission
    let start_pos = event_position(&events, "ToolCallStart").unwrap();
    let delta_pos = event_position(&events, "ToolCallDelta").unwrap();
    let end_pos = event_position(&events, "ToolCallEnd").unwrap();
    assert!(start_pos < delta_pos && delta_pos < end_pos);
}

// ===========================================================================
// 35. Anthropic — event ordering (TextDelta → Usage → Done)
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_event_ordering_text_usage_done() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_ord\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);

    // TextDelta must appear before Done
    let text_pos = event_position(&events, "TextDelta").expect("Expected TextDelta event");
    let done_pos = event_position(&events, "Done").expect("Expected Done event");
    assert!(
        text_pos < done_pos,
        "TextDelta (pos {text_pos}) should precede Done (pos {done_pos})"
    );

    // message_delta Usage appears right before Done (output tokens)
    let last_usage_pos = events
        .iter()
        .rposition(|e| matches!(e, AssistantMessageEvent::Usage(_)))
        .expect("Expected Usage event");
    assert!(
        last_usage_pos < done_pos,
        "Last Usage (pos {last_usage_pos}) should precede Done (pos {done_pos})"
    );
}

// ===========================================================================
// 36. Vertex AI — tool call stream
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_google_vertex_tool_call() {
    unsafe {
        std::env::set_var("GOOGLE_CLOUD_PROJECT", "test-project");
        std::env::set_var("GOOGLE_CLOUD_LOCATION", "us-central1");
    }

    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", mockito::Matcher::Regex(
            r"/v1/projects/test-project/locations/us-central1/publishers/google/models/gemini-2\.0-flash:streamGenerateContent\?.*".into(),
        ))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"functionCall\":{\"name\":\"bash\",\"args\":{\"command\":\"ls -la\"}}}]},\"finishReason\":null,\"index\":0}]}\n\n\
data: {\"candidates\":[{\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":12,\"candidatesTokenCount\":6,\"totalTokenCount\":18}}\n\n",
        )
        .create_async()
        .await;

    let provider = GoogleVertexProvider::new();
    let model = make_model(
        api::GOOGLE_VERTEX,
        &server.url(),
        "gemini-2.0-flash",
        "GOOGLE_CLOUD_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("run ls"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);

    // Vertex delegates to Google SSE parsing — verify full tool call lifecycle
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::ToolCallStart { name, .. } if name == "bash"
        )),
        "Expected ToolCallStart(bash)"
    );
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::ToolCallDelta { arguments_delta, .. }
            if arguments_delta.contains("ls -la")
        )),
        "Expected ToolCallDelta with 'ls -la'"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::ToolCallEnd { .. })),
        "Expected ToolCallEnd"
    );
    assert_has_done(&events);

    // Verify usage was also emitted
    let usages = collect_usage(&events);
    assert!(!usages.is_empty(), "Expected Usage from Vertex");
    assert_eq!(usages[0].input, 12);

    unsafe {
        std::env::remove_var("GOOGLE_CLOUD_PROJECT");
        std::env::remove_var("GOOGLE_CLOUD_LOCATION");
    }
}

// ===========================================================================
// 37. OpenAI Completions — ToolCallDelta lifecycle
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_completions_tool_call_delta() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"id\":\"chatcmpl-tc1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"call_tc1\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chatcmpl-tc1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\\\"\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chatcmpl-tc1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"/tmp\\\"}\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chatcmpl-tc1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n\
data: [DONE]\n\n",
        )
        .create_async()
        .await;

    let provider = OpenAiCompletionsProvider::new();
    let model = make_model(
        api::OPENAI_COMPLETIONS,
        &server.url(),
        "gpt-4",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("read /tmp"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);

    // ToolCallStart from first chunk (has name)
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::ToolCallStart { name, .. } if name == "read_file"
        )),
        "Expected ToolCallStart(read_file)"
    );

    // ToolCallDelta from subsequent chunks (arguments only, no name)
    let deltas: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AssistantMessageEvent::ToolCallDelta { .. }))
        .collect();
    assert!(
        !deltas.is_empty(),
        "Expected at least one ToolCallDelta event"
    );

    // Done with ToolUse stop reason (OpenAI Completions has no explicit ToolCallEnd)
    assert!(
        events.iter().any(|e| matches!(
            e,
            AssistantMessageEvent::Done { stop_reason }
            if *stop_reason == crate::types::StopReason::ToolUse
        )),
        "Expected Done with StopReason::ToolUse"
    );

    // ToolCallStart must precede ToolCallDelta
    let start_pos = event_position(&events, "ToolCallStart").unwrap();
    let delta_pos = event_position(&events, "ToolCallDelta").unwrap();
    assert!(start_pos < delta_pos, "Start before Delta");
}

// ===========================================================================
// 38. Anthropic — multi-turn conversation context
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_multi_turn_context() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_mt1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":50,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"The file contains hello.\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":10}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );

    // Multi-turn context: User → Assistant (tool call) → Tool result → User follow-up
    let context = LlmContext {
        messages: vec![
            LlmMessage::User {
                content: vec![LlmContent::Text("read /tmp/test.txt".into())],
            },
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![LlmToolCall {
                    id: "toolu_prev".into(),
                    function: LlmFunctionCall {
                        name: "read_file".into(),
                        arguments: r#"{"path":"/tmp/test.txt"}"#.into(),
                    },
                }],
                thinking_blocks: vec![],
            },
            LlmMessage::Tool {
                tool_call_id: "toolu_prev".into(),
                content: "hello".into(),
                tool_name: Some("read_file".into()),
            },
            LlmMessage::User {
                content: vec![LlmContent::Text("what does it contain?".into())],
            },
        ],
        system_prompt: "You are helpful.".into(),
        max_tokens: 4096,
        temperature: None,
    };

    let events = provider
        .stream(&model, &context, &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    assert!(collect_text(&events).contains("hello"));
    assert_has_done(&events);
    // Usage should reflect the larger input context (multi-turn)
    let usages = collect_usage(&events);
    assert!(!usages.is_empty(), "Expected Usage events");
    assert_eq!(usages[0].input, 50);
}

// ===========================================================================
// 39. Anthropic — thinking block (ThinkingDelta + ThinkingBlockEnd)
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_thinking_block() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_tk1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"Let me reason...\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig_abc123\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"The answer.\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":1}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":15}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("think about this"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);

    // ThinkingDelta emitted
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::ThinkingDelta(s) if s.contains("Let me reason")
        )),
        "Expected ThinkingDelta with 'Let me reason'"
    );

    // ThinkingBlockEnd with signature, not redacted
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::ThinkingBlockEnd { signature, redacted }
            if signature == "sig_abc123" && !redacted
        )),
        "Expected ThinkingBlockEnd(sig_abc123, redacted=false)"
    );

    // Text content still present
    assert!(collect_text(&events).contains("The answer."));
    assert_has_done(&events);
}

// ===========================================================================
// 40. Anthropic — redacted thinking block
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_redacted_thinking_block() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_tk2\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"redacted_thinking\",\"data\":\"encrypted_payload_base64\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Result.\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":1}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("think"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);

    // Redacted thinking emits placeholder ThinkingDelta
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::ThinkingDelta(s) if s.contains("redacted")
        )),
        "Expected ThinkingDelta with redacted placeholder"
    );

    // ThinkingBlockEnd with encrypted payload and redacted=true
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::ThinkingBlockEnd { signature, redacted }
            if signature == "encrypted_payload_base64" && *redacted
        )),
        "Expected ThinkingBlockEnd(encrypted_payload_base64, redacted=true)"
    );

    assert!(collect_text(&events).contains("Result."));
    assert_has_done(&events);
}

// ===========================================================================
// 41. OpenAI Completions — reasoning_content (ThinkingDelta)
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_completions_reasoning_content() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"id\":\"chatcmpl-r1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"deepseek-r1\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"Step 1: analyze...\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chatcmpl-r1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"deepseek-r1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"The answer is 42.\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chatcmpl-r1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"deepseek-r1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
data: [DONE]\n\n",
        )
        .create_async()
        .await;

    let provider = OpenAiCompletionsProvider::new();
    let model = make_model(
        api::OPENAI_COMPLETIONS,
        &server.url(),
        "deepseek-r1",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("think"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);

    // reasoning_content → ThinkingDelta
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::ThinkingDelta(s) if s.contains("Step 1")
        )),
        "Expected ThinkingDelta from reasoning_content"
    );

    // Regular content → TextDelta
    assert!(collect_text(&events).contains("The answer is 42."));
    assert_has_done(&events);
}

// ===========================================================================
// 42. Google — thinking part (thought: true)
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_google_thinking_part() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", mockito::Matcher::Regex(
            r"/models/gemini-2\.0-flash:streamGenerateContent\?.*".into(),
        ))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Let me think...\",\"thought\":true}]},\"finishReason\":null,\"index\":0}]}\n\n\
data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"The answer.\"}]},\"finishReason\":null,\"index\":0}]}\n\n\
data: {\"candidates\":[{\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":5,\"totalTokenCount\":15}}\n\n",
        )
        .create_async()
        .await;

    let provider = GoogleProvider::new();
    let model = make_model(
        api::GOOGLE_GENERATIVE_AI,
        &server.url(),
        "gemini-2.0-flash",
        "GOOGLE_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("think"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);

    // thought:true → ThinkingDelta
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::ThinkingDelta(s) if s.contains("Let me think")
        )),
        "Expected ThinkingDelta from thought:true part"
    );

    // Regular part → TextDelta
    assert!(collect_text(&events).contains("The answer."));
    assert_has_done(&events);
}

// ===========================================================================
// 43. OpenAI Responses — reasoning_summary_text.delta (ThinkingDelta)
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_responses_reasoning_summary() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/responses")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
event: response.reasoning_summary_text.delta\n\
data: {\"delta\":\"Step 1: analyze the problem.\"}\n\n\
event: response.output_item.added\n\
data: {\"item\":{\"type\":\"message\",\"id\":\"item_r1\",\"index\":0,\"status\":\"in_progress\"}}\n\n\
event: response.output_text.delta\n\
data: {\"delta\":\"The answer.\",\"item_index\":0}\n\n\
event: response.output_item.done\n\
data: {\"item\":{\"type\":\"message\",\"id\":\"item_r1\",\"index\":0,\"status\":\"completed\"}}\n\n\
event: response.completed\n\
data: {\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}\n\n",
        )
        .create_async()
        .await;

    let provider = OpenAiResponsesProvider::new();
    let model = make_model(
        api::OPENAI_RESPONSES,
        &server.url(),
        "gpt-4o",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("think"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);

    // reasoning_summary_text.delta → ThinkingDelta
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::ThinkingDelta(s) if s.contains("Step 1")
        )),
        "Expected ThinkingDelta from reasoning_summary_text.delta"
    );

    assert!(collect_text(&events).contains("The answer."));
    assert_has_done(&events);
}

// ===========================================================================
// 44. Anthropic — 500 internal server error
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_error_500() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(500)
        .with_body("{\"error\":{\"message\":\"Internal server error\"}}")
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Error(m) if m.contains("500"))),
        "Expected 500 error, got: {events:?}"
    );
}

// ===========================================================================
// 45. Google — 500 internal server error
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_google_error_500() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock(
            "POST",
            mockito::Matcher::Regex(r"/models/gemini-2\.0-flash:streamGenerateContent\?.*".into()),
        )
        .with_status(500)
        .with_body("{\"error\":{\"message\":\"Internal error\"}}")
        .create_async()
        .await;

    let provider = GoogleProvider::new();
    let model = make_model(
        api::GOOGLE_GENERATIVE_AI,
        &server.url(),
        "gemini-2.0-flash",
        "GOOGLE_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("bad-key"))
        .await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Error(m) if m.contains("500"))),
        "Expected 500 error, got: {events:?}"
    );
}

// ===========================================================================
// 46. Azure — 500 internal server error
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_azure_error_500() {
    unsafe {
        std::env::remove_var("AZURE_OPENAI_BASE_URL");
        std::env::remove_var("AZURE_OPENAI_RESOURCE_NAME");
        std::env::remove_var("AZURE_OPENAI_DEPLOYMENT_NAME_MAP");
        std::env::remove_var("AZURE_OPENAI_API_VERSION");
    }

    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock(
            "POST",
            mockito::Matcher::Regex(r"/responses\?api-version=.*".into()),
        )
        .with_status(500)
        .with_body("<html>Internal Server Error</html>")
        .create_async()
        .await;

    let provider = AzureOpenAiResponsesProvider::new();
    let model = make_model(
        api::AZURE_OPENAI_RESPONSES,
        &server.url(),
        "gpt-4o",
        "AZURE_OPENAI_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Error(m) if m.contains("500"))),
        "Expected 500 error, got: {events:?}"
    );
}

// ===========================================================================
// 47. OpenAI Completions — missing API key
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_completions_missing_api_key() {
    ensure_no_proxy();
    let provider = OpenAiCompletionsProvider::new();
    let model = make_model(
        api::OPENAI_COMPLETIONS,
        "http://localhost:1",
        "gpt-4",
        "NONEXISTENT_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &StreamOptions::default())
        .await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Error(..))),
        "Expected error for missing API key, got: {events:?}"
    );
}

// ===========================================================================
// 48. Google — missing API key
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_google_missing_api_key() {
    ensure_no_proxy();
    let provider = GoogleProvider::new();
    let model = make_model(
        api::GOOGLE_GENERATIVE_AI,
        "http://localhost:1",
        "gemini-2.0-flash",
        "NONEXISTENT_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &StreamOptions::default())
        .await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Error(..))),
        "Expected error for missing API key, got: {events:?}"
    );
}

// ===========================================================================
// 49. Anthropic — truncated SSE stream (no message_delta/Done)
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_truncated_stream() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_tr1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial output\"}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    // Should not panic — partial text should still be captured
    assert!(collect_text(&events).contains("partial output"));
    // No Done event expected (stream was truncated)
}

// ===========================================================================
// 50. OpenAI Completions — empty body 200 response
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_completions_empty_body_200() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body("")
        .create_async()
        .await;

    let provider = OpenAiCompletionsProvider::new();
    let model = make_model(
        api::OPENAI_COMPLETIONS,
        &server.url(),
        "gpt-4",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    // Should not panic — should get empty or error events
    assert_no_errors(&events);
    assert!(collect_text(&events).is_empty());
}

// ===========================================================================
// 51. Anthropic — multiple tool calls in same response
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_multiple_tool_calls() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_mt2\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_a\",\"name\":\"bash\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"cmd\\\":\\\"ls\\\"}\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_b\",\"name\":\"read_file\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"/tmp\\\"}\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":1}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":20}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("do two things"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);

    // Two distinct ToolCallStart events
    let starts: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AssistantMessageEvent::ToolCallStart { .. }))
        .collect();
    assert_eq!(
        starts.len(),
        2,
        "Expected 2 ToolCallStart events, got {}",
        starts.len()
    );

    // Verify both tool names present
    assert!(events.iter().any(|e| matches!(
        e, AssistantMessageEvent::ToolCallStart { name, .. } if name == "bash"
    )));
    assert!(events.iter().any(|e| matches!(
        e, AssistantMessageEvent::ToolCallStart { name, .. } if name == "read_file"
    )));

    // Two ToolCallEnd events
    let ends: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AssistantMessageEvent::ToolCallEnd { .. }))
        .collect();
    assert_eq!(
        ends.len(),
        2,
        "Expected 2 ToolCallEnd events, got {}",
        ends.len()
    );

    // Done with ToolUse
    assert!(events.iter().any(|e| matches!(
        e, AssistantMessageEvent::Done { stop_reason }
        if *stop_reason == crate::types::StopReason::ToolUse
    )));
}

// ===========================================================================
// 52. Anthropic — stop_reason tool_use → ToolUse (explicit)
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_stop_reason_tool_use() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_sr1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_sr1\",\"name\":\"bash\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":5}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(
            &model,
            &make_context("run it"),
            &[],
            &make_options("test-key"),
        )
        .await;

    assert_no_errors(&events);
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::Done { stop_reason }
            if *stop_reason == crate::types::StopReason::ToolUse
        )),
        "Expected Done with StopReason::ToolUse for stop_reason=tool_use"
    );
}

// ===========================================================================
// 53. Google — stop_reason STOP explicit mapping
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_google_stop_reason_stop() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", mockito::Matcher::Regex(
            r"/models/gemini-2\.0-flash:streamGenerateContent\?.*".into(),
        ))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Done.\"}]},\"finishReason\":null,\"index\":0}]}\n\n\
data: {\"candidates\":[{\"finishReason\":\"STOP\",\"index\":0}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":2,\"totalTokenCount\":12}}\n\n",
        )
        .create_async()
        .await;

    let provider = GoogleProvider::new();
    let model = make_model(
        api::GOOGLE_GENERATIVE_AI,
        &server.url(),
        "gemini-2.0-flash",
        "GOOGLE_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::Done { stop_reason }
            if *stop_reason == crate::types::StopReason::Stop
        )),
        "Expected Done with StopReason::Stop for STOP"
    );
}

// ===========================================================================
// 54. OpenAI Responses — response.failed in-stream error
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_openai_responses_stream_failed_event() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/responses")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
event: response.output_item.added\n\
data: {\"item\":{\"type\":\"message\",\"id\":\"item_f1\",\"index\":0,\"status\":\"in_progress\"}}\n\n\
event: response.output_text.delta\n\
data: {\"delta\":\"partial\",\"item_index\":0}\n\n\
event: response.failed\n\
data: {\"response\":{\"status\":\"failed\",\"error\":{\"code\":\"server_error\",\"message\":\"Internal failure\"}}}\n\n",
        )
        .create_async()
        .await;

    let provider = OpenAiResponsesProvider::new();
    let model = make_model(
        api::OPENAI_RESPONSES,
        &server.url(),
        "gpt-4o",
        "OPENAI_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    // Partial text should still be present
    assert!(collect_text(&events).contains("partial"));
    // Error event from response.failed
    assert!(
        events.iter().any(|e| matches!(
            e, AssistantMessageEvent::Error(m) if m.contains("server_error")
        )),
        "Expected Error event from response.failed"
    );
}

// ===========================================================================
// 55. Anthropic — unicode/emoji in text delta
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_anthropic_unicode_text_delta() {
    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_uc1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello \\ud83c\\udf0d 世界 café naïve\"}}\n\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n",
        )
        .create_async()
        .await;

    let provider = AnthropicProvider::new();
    let model = make_model(
        api::ANTHROPIC_MESSAGES,
        &server.url(),
        "claude-sonnet",
        "ANTHROPIC_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    let text = collect_text(&events);
    assert!(text.contains("世界"), "Expected CJK characters in text");
    assert!(
        text.contains("café"),
        "Expected accented characters in text"
    );
    assert_has_done(&events);
}

// ===========================================================================
// 56. Azure — Usage event verification
// ===========================================================================

#[tokio::test]
#[serial]
async fn integration_azure_usage_event_tokens() {
    unsafe {
        std::env::remove_var("AZURE_OPENAI_BASE_URL");
        std::env::remove_var("AZURE_OPENAI_RESOURCE_NAME");
        std::env::remove_var("AZURE_OPENAI_DEPLOYMENT_NAME_MAP");
        std::env::remove_var("AZURE_OPENAI_API_VERSION");
    }

    ensure_no_proxy();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", mockito::Matcher::Regex(r"/responses\?api-version=.*".into()))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(
            "\
event: response.output_item.added\n\
data: {\"item\":{\"type\":\"message\",\"id\":\"item_au1\",\"index\":0,\"status\":\"in_progress\"}}\n\n\
event: response.output_text.delta\n\
data: {\"delta\":\"Hi\",\"item_index\":0}\n\n\
event: response.output_item.done\n\
data: {\"item\":{\"type\":\"message\",\"id\":\"item_au1\",\"index\":0,\"status\":\"completed\"}}\n\n\
event: response.completed\n\
data: {\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":25,\"output_tokens\":10}}}\n\n",
        )
        .create_async()
        .await;

    let provider = AzureOpenAiResponsesProvider::new();
    let model = make_model(
        api::AZURE_OPENAI_RESPONSES,
        &server.url(),
        "gpt-4o",
        "AZURE_OPENAI_API_KEY",
    );
    let events = provider
        .stream(&model, &make_context("hi"), &[], &make_options("test-key"))
        .await;

    assert_no_errors(&events);
    let usages = collect_usage(&events);
    assert!(!usages.is_empty(), "Expected Usage event from Azure");
    assert_eq!(usages[0].input, 25);
    assert_eq!(usages[0].output, 10);
    assert_eq!(usages[0].total_tokens, 35);
}
