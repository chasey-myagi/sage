// Amazon Bedrock ConverseStream Provider
// Wraps the AWS Bedrock Runtime ConverseStream API with credential chain
// resolution, message format conversion, and thinking/cache support.
//
// Pi-mono reference: providers/amazon-bedrock.ts
//
// Key differences from OpenAI-compatible providers:
// - Auth: AWS SigV4 via SDK credential chain (not API key header)
// - Protocol: ConverseStream (binary event stream, not SSE)
// - Message format: Bedrock-native Content/ContentBlock (not OpenAI chat)
// - Tool call IDs: 64-char alphanumeric sanitization required
// - Thinking: reasoningContent with optional signature (Claude-only)
// - Cache: cache points on system prompt + last user message (Claude 3.5+)

use crate::registry::{ApiProvider, StreamOptions};
use crate::types::*;
use async_trait::async_trait;
use aws_sdk_bedrockruntime as bedrock;
use aws_sdk_bedrockruntime::types as br;
use aws_smithy_types::Document;

// ---------------------------------------------------------------------------
// Provider struct
// ---------------------------------------------------------------------------

/// Provider for the Amazon Bedrock ConverseStream API.
///
/// Uses the AWS SDK for credential resolution and request signing.
/// Supports Claude, Llama, Nova, and other models via Bedrock.
#[derive(Default)]
pub struct BedrockProvider;

impl BedrockProvider {
    pub fn new() -> Self {
        Self
    }
}

// ---------------------------------------------------------------------------
// Configuration helpers
// ---------------------------------------------------------------------------

/// Resolves the AWS region for Bedrock.
/// Priority: AWS_REGION > AWS_DEFAULT_REGION > us-east-1 (unless AWS_PROFILE set)
fn resolve_region() -> Option<String> {
    if let Some(r) = std::env::var("AWS_REGION").ok().filter(|s| !s.is_empty()) {
        return Some(r);
    }
    if let Some(r) = std::env::var("AWS_DEFAULT_REGION")
        .ok()
        .filter(|s| !s.is_empty())
    {
        return Some(r);
    }
    // If AWS_PROFILE is set, let SDK resolve region from profile config
    if std::env::var("AWS_PROFILE").is_ok() {
        return None;
    }
    Some("us-east-1".into())
}

/// Sanitize tool call ID to 64-char alphanumeric (Bedrock requirement).
fn normalize_tool_call_id(id: &str) -> String {
    let sanitized: String = id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.len() > 64 {
        sanitized[..64].to_string()
    } else {
        sanitized
    }
}

/// Check if model supports thinking signatures (Claude-only).
fn supports_thinking_signature(model_id: &str) -> bool {
    let id = model_id.to_lowercase();
    id.contains("anthropic.claude") || id.contains("anthropic/claude")
}

/// Convert serde_json::Value to aws_smithy_types::Document.
fn json_to_document(value: &serde_json::Value) -> Document {
    match value {
        serde_json::Value::Null => Document::Null,
        serde_json::Value::Bool(b) => Document::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= 0 {
                    Document::Number(aws_smithy_types::Number::PosInt(i as u64))
                } else {
                    Document::Number(aws_smithy_types::Number::NegInt(i))
                }
            } else if let Some(f) = n.as_f64() {
                Document::Number(aws_smithy_types::Number::Float(f))
            } else {
                Document::Null
            }
        }
        serde_json::Value::String(s) => Document::String(s.clone()),
        serde_json::Value::Array(arr) => {
            Document::Array(arr.iter().map(json_to_document).collect())
        }
        serde_json::Value::Object(obj) => Document::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), json_to_document(v)))
                .collect(),
        ),
    }
}

/// Check if model supports prompt caching.
fn supports_prompt_caching(model_id: &str) -> bool {
    let id = model_id.to_lowercase();
    if !id.contains("claude") {
        return std::env::var("AWS_BEDROCK_FORCE_CACHE")
            .map(|v| v == "1")
            .unwrap_or(false);
    }
    // Claude 4.x
    if id.contains("-4-") || id.contains("-4.") {
        return true;
    }
    // Claude 3.7 Sonnet
    if id.contains("claude-3-7-sonnet") {
        return true;
    }
    // Claude 3.5 Haiku
    if id.contains("claude-3-5-haiku") {
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Message conversion: LlmMessage → Bedrock Message
// ---------------------------------------------------------------------------

/// Convert LlmContext messages to Bedrock Message format.
fn convert_messages(context: &LlmContext, model: &Model) -> Vec<br::Message> {
    let mut result = Vec::new();
    let messages = &context.messages;
    let mut i = 0;

    while i < messages.len() {
        match &messages[i] {
            LlmMessage::System { .. } => {
                // System messages handled separately in build_system_prompt
                i += 1;
            }
            LlmMessage::User { content } => {
                let blocks: Vec<br::ContentBlock> = content
                    .iter()
                    .map(|c| match c {
                        LlmContent::Text(text) => br::ContentBlock::Text(text.clone()),
                        LlmContent::Image { url } => {
                            // For now, images passed as text description
                            // Full base64 image support would need MIME type parsing
                            br::ContentBlock::Text(format!("[image: {}]", url))
                        }
                    })
                    .collect();
                result.push(
                    br::Message::builder()
                        .role(br::ConversationRole::User)
                        .set_content(Some(blocks))
                        .build()
                        .expect("valid message"),
                );
                i += 1;
            }
            LlmMessage::Assistant {
                content,
                tool_calls,
                thinking_blocks,
            } => {
                let mut blocks: Vec<br::ContentBlock> = Vec::new();

                // Add thinking blocks
                for tb in thinking_blocks {
                    if tb.thinking.trim().is_empty() {
                        continue;
                    }
                    if supports_thinking_signature(&model.id) {
                        if let Some(sig) = tb.signature.as_ref().filter(|s| !s.trim().is_empty()) {
                            blocks.push(br::ContentBlock::ReasoningContent(
                                br::ReasoningContentBlock::ReasoningText(
                                    br::ReasoningTextBlock::builder()
                                        .text(tb.thinking.clone())
                                        .signature(sig.clone())
                                        .build()
                                        .expect("valid reasoning text"),
                                ),
                            ));
                        } else {
                            // No signature — fall back to plain text
                            blocks.push(br::ContentBlock::Text(tb.thinking.clone()));
                        }
                    } else {
                        blocks.push(br::ContentBlock::ReasoningContent(
                            br::ReasoningContentBlock::ReasoningText(
                                br::ReasoningTextBlock::builder()
                                    .text(tb.thinking.clone())
                                    .build()
                                    .expect("valid reasoning text"),
                            ),
                        ));
                    }
                }

                // Add text content
                if !content.is_empty() {
                    blocks.push(br::ContentBlock::Text(content.clone()));
                }

                // Add tool calls
                for tc in tool_calls {
                    let input: serde_json::Value =
                        serde_json::from_str(&tc.function.arguments).unwrap_or_default();
                    blocks.push(br::ContentBlock::ToolUse(
                        br::ToolUseBlock::builder()
                            .tool_use_id(normalize_tool_call_id(&tc.id))
                            .name(tc.function.name.clone())
                            .input(json_to_document(&input))
                            .build()
                            .expect("valid tool use"),
                    ));
                }

                if !blocks.is_empty() {
                    result.push(
                        br::Message::builder()
                            .role(br::ConversationRole::Assistant)
                            .set_content(Some(blocks))
                            .build()
                            .expect("valid message"),
                    );
                }
                i += 1;
            }
            LlmMessage::Tool {
                tool_call_id,
                content,
                ..
            } => {
                // Collect consecutive tool results into single user message
                let mut tool_results: Vec<br::ContentBlock> = Vec::new();

                tool_results.push(br::ContentBlock::ToolResult(
                    br::ToolResultBlock::builder()
                        .tool_use_id(normalize_tool_call_id(tool_call_id))
                        .content(br::ToolResultContentBlock::Text(content.clone()))
                        .status(br::ToolResultStatus::Success)
                        .build()
                        .expect("valid tool result"),
                ));

                // Look ahead for consecutive tool results
                let mut j = i + 1;
                while j < messages.len() {
                    if let LlmMessage::Tool {
                        tool_call_id: next_id,
                        content: next_content,
                        ..
                    } = &messages[j]
                    {
                        tool_results.push(br::ContentBlock::ToolResult(
                            br::ToolResultBlock::builder()
                                .tool_use_id(normalize_tool_call_id(next_id))
                                .content(br::ToolResultContentBlock::Text(next_content.clone()))
                                .status(br::ToolResultStatus::Success)
                                .build()
                                .expect("valid tool result"),
                        ));
                        j += 1;
                    } else {
                        break;
                    }
                }

                result.push(
                    br::Message::builder()
                        .role(br::ConversationRole::User)
                        .set_content(Some(tool_results))
                        .build()
                        .expect("valid message"),
                );
                i = j;
            }
        }
    }

    // Add cache point to last user message for supported models
    if supports_prompt_caching(&model.id)
        && let Some(last) = result
            .last_mut()
            .filter(|m| m.role() == &br::ConversationRole::User)
    {
        last.content.push(br::ContentBlock::CachePoint(
            br::CachePointBlock::builder()
                .r#type(br::CachePointType::Default)
                .build()
                .expect("valid cache point"),
        ));
    }

    result
}

/// Build system prompt blocks.
fn build_system_prompt(system_prompt: &str, model: &Model) -> Option<Vec<br::SystemContentBlock>> {
    if system_prompt.is_empty() {
        return None;
    }
    let mut blocks = vec![br::SystemContentBlock::Text(system_prompt.to_string())];

    if supports_prompt_caching(&model.id) {
        blocks.push(br::SystemContentBlock::CachePoint(
            br::CachePointBlock::builder()
                .r#type(br::CachePointType::Default)
                .build()
                .expect("valid cache point"),
        ));
    }

    Some(blocks)
}

/// Convert tools to Bedrock ToolConfiguration.
fn convert_tool_config(tools: &[LlmTool]) -> Option<br::ToolConfiguration> {
    if tools.is_empty() {
        return None;
    }

    let bedrock_tools: Vec<br::Tool> = tools
        .iter()
        .map(|t| {
            br::Tool::ToolSpec(
                br::ToolSpecification::builder()
                    .name(t.name.clone())
                    .description(t.description.clone())
                    .input_schema(br::ToolInputSchema::Json(json_to_document(&t.parameters)))
                    .build()
                    .expect("valid tool spec"),
            )
        })
        .collect();

    Some(
        br::ToolConfiguration::builder()
            .set_tools(Some(bedrock_tools))
            .build()
            .expect("valid tool config"),
    )
}

/// Map Bedrock stop reason to our StopReason.
fn map_stop_reason(reason: &br::StopReason) -> crate::types::StopReason {
    match reason {
        br::StopReason::EndTurn | br::StopReason::StopSequence => crate::types::StopReason::Stop,
        br::StopReason::MaxTokens | br::StopReason::ModelContextWindowExceeded => {
            crate::types::StopReason::Length
        }
        br::StopReason::ToolUse => crate::types::StopReason::ToolUse,
        _ => crate::types::StopReason::Stop,
    }
}

// ---------------------------------------------------------------------------
// ApiProvider implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ApiProvider for BedrockProvider {
    fn api(&self) -> &str {
        api::BEDROCK_CONVERSE_STREAM
    }

    async fn stream(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
        _options: &StreamOptions,
    ) -> Vec<AssistantMessageEvent> {
        let mut events = Vec::new();

        // Build AWS config
        let mut config_loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
        if let Some(region) = resolve_region() {
            config_loader = config_loader.region(aws_types::region::Region::new(region));
        }

        // Support proxies that skip auth
        if std::env::var("AWS_BEDROCK_SKIP_AUTH")
            .map(|v| v == "1")
            .unwrap_or(false)
        {
            config_loader = config_loader.credentials_provider(bedrock::config::Credentials::new(
                "dummy-access-key",
                "dummy-secret-key",
                None,
                None,
                "skip-auth",
            ));
        }

        let sdk_config = config_loader.load().await;
        let client = bedrock::Client::new(&sdk_config);

        // Build ConverseStream command
        let messages = convert_messages(context, model);
        let system_prompt = build_system_prompt(&context.system_prompt, model);
        let tool_config = convert_tool_config(tools);

        let mut cmd = client
            .converse_stream()
            .model_id(&model.id)
            .set_messages(Some(messages));

        if let Some(sys) = system_prompt {
            cmd = cmd.set_system(Some(sys));
        }

        if let Some(tc) = tool_config {
            cmd = cmd.tool_config(tc);
        }

        // Set inference config
        let mut inf_config =
            br::InferenceConfiguration::builder().max_tokens(context.max_tokens as i32);
        if let Some(temp) = context.temperature {
            inf_config = inf_config.temperature(temp);
        }
        cmd = cmd.inference_config(inf_config.build());

        // Send request
        let response = match cmd.send().await {
            Ok(r) => r,
            Err(e) => {
                return vec![AssistantMessageEvent::Error(format!(
                    "Bedrock ConverseStream failed: {e}"
                ))];
            }
        };

        // Process event stream
        let mut stream = response.stream;
        let mut current_tool_id = String::new();
        let mut stop_reason = crate::types::StopReason::Stop;

        loop {
            match stream.recv().await {
                Ok(Some(event)) => match event {
                    bedrock::types::ConverseStreamOutput::ContentBlockDelta(delta_event) => {
                        if let Some(delta) = delta_event.delta() {
                            match delta {
                                br::ContentBlockDelta::Text(text) => {
                                    events.push(AssistantMessageEvent::TextDelta(text.to_string()));
                                }
                                br::ContentBlockDelta::ToolUse(tool_delta) => {
                                    let input = tool_delta.input();
                                    if !input.is_empty() {
                                        events.push(AssistantMessageEvent::ToolCallDelta {
                                            id: current_tool_id.clone(),
                                            arguments_delta: input.to_string(),
                                        });
                                    }
                                }
                                br::ContentBlockDelta::ReasoningContent(rc) => match rc {
                                    br::ReasoningContentBlockDelta::Text(text) => {
                                        events.push(AssistantMessageEvent::ThinkingDelta(
                                            text.clone(),
                                        ));
                                    }
                                    br::ReasoningContentBlockDelta::Signature(sig) => {
                                        events.push(AssistantMessageEvent::ThinkingBlockEnd {
                                            signature: sig.clone(),
                                            redacted: false,
                                        });
                                    }
                                    br::ReasoningContentBlockDelta::RedactedContent(_) => {
                                        events.push(AssistantMessageEvent::ThinkingBlockEnd {
                                            signature: String::new(),
                                            redacted: true,
                                        });
                                    }
                                    _ => {}
                                },
                                _ => {}
                            }
                        }
                    }
                    bedrock::types::ConverseStreamOutput::ContentBlockStart(start_event) => {
                        if let Some(tool_use) =
                            start_event.start().and_then(|s| s.as_tool_use().ok())
                        {
                            current_tool_id = tool_use.tool_use_id().to_string();
                            let name = tool_use.name().to_string();
                            events.push(AssistantMessageEvent::ToolCallStart {
                                id: current_tool_id.clone(),
                                name,
                            });
                        }
                    }
                    bedrock::types::ConverseStreamOutput::ContentBlockStop(_) => {
                        if !current_tool_id.is_empty() {
                            events.push(AssistantMessageEvent::ToolCallEnd {
                                id: current_tool_id.clone(),
                            });
                            current_tool_id.clear();
                        }
                    }
                    bedrock::types::ConverseStreamOutput::MessageStop(stop_event) => {
                        stop_reason = map_stop_reason(stop_event.stop_reason());
                    }
                    bedrock::types::ConverseStreamOutput::Metadata(meta) => {
                        if let Some(usage) = meta.usage() {
                            events.push(AssistantMessageEvent::Usage(crate::types::Usage {
                                input: usage.input_tokens() as u64,
                                output: usage.output_tokens() as u64,
                                cache_read: usage.cache_read_input_tokens().unwrap_or(0) as u64,
                                cache_write: usage.cache_write_input_tokens().unwrap_or(0) as u64,
                                ..Default::default()
                            }));
                        }
                    }
                    _ => {}
                },
                Ok(None) => break,
                Err(e) => {
                    events.push(AssistantMessageEvent::Error(format!(
                        "Bedrock stream error: {e}"
                    )));
                    break;
                }
            }
        }

        events.push(AssistantMessageEvent::Done { stop_reason });
        events
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    // ========================================================================
    // normalize_tool_call_id
    // ========================================================================

    #[test]
    fn test_normalize_tool_call_id_passthrough() {
        assert_eq!(normalize_tool_call_id("call_abc123"), "call_abc123");
    }

    #[test]
    fn test_normalize_tool_call_id_special_chars() {
        assert_eq!(normalize_tool_call_id("call.foo@bar"), "call_foo_bar");
    }

    #[test]
    fn test_normalize_tool_call_id_truncate() {
        let long_id = "a".repeat(100);
        let result = normalize_tool_call_id(&long_id);
        assert_eq!(result.len(), 64);
    }

    #[test]
    fn test_normalize_tool_call_id_short() {
        assert_eq!(normalize_tool_call_id("x"), "x");
    }

    #[test]
    fn test_normalize_tool_call_id_hyphens_preserved() {
        assert_eq!(normalize_tool_call_id("call-foo-bar"), "call-foo-bar");
    }

    // ========================================================================
    // supports_thinking_signature
    // ========================================================================

    #[test]
    fn test_supports_thinking_signature_claude() {
        assert!(supports_thinking_signature("anthropic.claude-opus-4-6-v1"));
        assert!(supports_thinking_signature("anthropic.claude-sonnet-4-6"));
        assert!(supports_thinking_signature(
            "eu.anthropic.claude-opus-4-6-v1"
        ));
    }

    #[test]
    fn test_supports_thinking_signature_non_claude() {
        assert!(!supports_thinking_signature("amazon.nova-pro"));
        assert!(!supports_thinking_signature("meta.llama3-2-90b"));
        assert!(!supports_thinking_signature("qwen.qwen3-235b"));
    }

    // ========================================================================
    // supports_prompt_caching
    // ========================================================================

    #[test]
    fn test_supports_prompt_caching_claude4() {
        assert!(supports_prompt_caching("anthropic.claude-opus-4-6-v1"));
        assert!(supports_prompt_caching("anthropic.claude-sonnet-4-6"));
    }

    #[test]
    fn test_supports_prompt_caching_claude37() {
        assert!(supports_prompt_caching(
            "anthropic.claude-3-7-sonnet-20250219-v1:0"
        ));
    }

    #[test]
    fn test_supports_prompt_caching_claude35_haiku() {
        assert!(supports_prompt_caching(
            "anthropic.claude-3-5-haiku-20241022-v1:0"
        ));
    }

    #[test]
    #[serial]
    fn test_supports_prompt_caching_non_claude() {
        unsafe { std::env::remove_var("AWS_BEDROCK_FORCE_CACHE") };
        assert!(!supports_prompt_caching("amazon.nova-pro-v1:0"));
        assert!(!supports_prompt_caching("meta.llama3-2-90b"));
    }

    // ========================================================================
    // resolve_region
    // ========================================================================

    #[test]
    #[serial]
    fn test_resolve_region_default_us_east_1() {
        unsafe { std::env::remove_var("AWS_REGION") };
        unsafe { std::env::remove_var("AWS_DEFAULT_REGION") };
        unsafe { std::env::remove_var("AWS_PROFILE") };
        assert_eq!(resolve_region(), Some("us-east-1".into()));
    }

    #[test]
    #[serial]
    fn test_resolve_region_from_env() {
        unsafe { std::env::set_var("AWS_REGION", "eu-west-1") };
        assert_eq!(resolve_region(), Some("eu-west-1".into()));
        unsafe { std::env::remove_var("AWS_REGION") };
    }

    #[test]
    #[serial]
    fn test_resolve_region_none_when_profile_set() {
        unsafe { std::env::remove_var("AWS_REGION") };
        unsafe { std::env::remove_var("AWS_DEFAULT_REGION") };
        unsafe { std::env::set_var("AWS_PROFILE", "my-profile") };
        assert_eq!(resolve_region(), None);
        unsafe { std::env::remove_var("AWS_PROFILE") };
    }

    // ========================================================================
    // map_stop_reason
    // ========================================================================

    #[test]
    fn test_map_stop_reason_end_turn() {
        assert!(matches!(
            map_stop_reason(&br::StopReason::EndTurn),
            crate::types::StopReason::Stop
        ));
    }

    #[test]
    fn test_map_stop_reason_max_tokens() {
        assert!(matches!(
            map_stop_reason(&br::StopReason::MaxTokens),
            crate::types::StopReason::Length
        ));
    }

    #[test]
    fn test_map_stop_reason_tool_use() {
        assert!(matches!(
            map_stop_reason(&br::StopReason::ToolUse),
            crate::types::StopReason::ToolUse
        ));
    }

    // ========================================================================
    // convert_tool_config
    // ========================================================================

    #[test]
    fn test_convert_tool_config_empty() {
        assert!(convert_tool_config(&[]).is_none());
    }

    #[test]
    fn test_convert_tool_config_single_tool() {
        let tools = vec![LlmTool {
            name: "bash".into(),
            description: "Run a command".into(),
            parameters: serde_json::json!({"type": "object"}),
        }];
        let config = convert_tool_config(&tools);
        assert!(config.is_some());
        let config = config.unwrap();
        assert_eq!(config.tools().len(), 1);
    }

    // ========================================================================
    // Provider identity
    // ========================================================================

    #[test]
    fn test_provider_api_identifier() {
        let provider = BedrockProvider::new();
        assert_eq!(provider.api(), "bedrock-converse-stream");
    }

    // ========================================================================
    // build_system_prompt
    // ========================================================================

    #[test]
    fn test_build_system_prompt_empty() {
        let model = crate::test_helpers::test_model();
        assert!(build_system_prompt("", &model).is_none());
    }

    #[test]
    fn test_build_system_prompt_non_empty() {
        let model = crate::test_helpers::test_model();
        let result = build_system_prompt("You are helpful.", &model);
        assert!(result.is_some());
        assert!(!result.unwrap().is_empty());
    }

    // ========================================================================
    // convert_messages — basic
    // ========================================================================

    #[test]
    fn test_convert_messages_user_only() {
        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("hello".into())],
            }],
            system_prompt: String::new(),
            max_tokens: 4096,
            temperature: None,
        };
        let model = crate::test_helpers::test_model();
        let messages = convert_messages(&context, &model);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role(), &br::ConversationRole::User);
    }

    #[test]
    fn test_convert_messages_tool_results_aggregated() {
        let context = LlmContext {
            messages: vec![
                LlmMessage::User {
                    content: vec![LlmContent::Text("run two commands".into())],
                },
                LlmMessage::Assistant {
                    content: String::new(),
                    tool_calls: vec![
                        LlmToolCall {
                            id: "call_1".into(),
                            function: LlmFunctionCall {
                                name: "bash".into(),
                                arguments: r#"{"cmd":"ls"}"#.into(),
                            },
                        },
                        LlmToolCall {
                            id: "call_2".into(),
                            function: LlmFunctionCall {
                                name: "bash".into(),
                                arguments: r#"{"cmd":"pwd"}"#.into(),
                            },
                        },
                    ],
                    thinking_blocks: vec![],
                },
                LlmMessage::Tool {
                    tool_call_id: "call_1".into(),
                    content: "file1\nfile2".into(),
                    tool_name: None,
                },
                LlmMessage::Tool {
                    tool_call_id: "call_2".into(),
                    content: "/home".into(),
                    tool_name: None,
                },
            ],
            system_prompt: String::new(),
            max_tokens: 4096,
            temperature: None,
        };
        let model = crate::test_helpers::test_model();
        let messages = convert_messages(&context, &model);
        // user + assistant + user (aggregated tool results)
        assert_eq!(messages.len(), 3);
        // Last message should be user with 2 tool results
        assert_eq!(messages[2].role(), &br::ConversationRole::User);
        assert_eq!(messages[2].content().len(), 2);
    }
}
