// GitHub Copilot header utilities — ported from pi-mono's github-copilot-headers.ts.
// Provides helper functions for building Copilot-specific HTTP headers.

use crate::types::LlmMessage;

// ---------------------------------------------------------------------------
// Initiator inference
// ---------------------------------------------------------------------------

/// Infer whether the Copilot request is user-initiated or agent-initiated.
///
/// Copilot requires the `X-Initiator` header to distinguish between requests
/// triggered by the user (last message is a user message) and agent-initiated
/// follow-ups (last message is an assistant or tool message).
pub fn infer_copilot_initiator(messages: &[LlmMessage]) -> &'static str {
    match messages.last() {
        Some(LlmMessage::User { .. }) | None => "user",
        _ => "agent",
    }
}

// ---------------------------------------------------------------------------
// Vision detection
// ---------------------------------------------------------------------------

/// Check if any message contains image content.
///
/// Copilot requires the `Copilot-Vision-Request: true` header when the
/// request contains image inputs.
pub fn has_copilot_vision_input(messages: &[LlmMessage]) -> bool {
    messages.iter().any(|msg| match msg {
        LlmMessage::User { content } => content
            .iter()
            .any(|c| matches!(c, crate::types::LlmContent::Image { .. })),
        // Tool messages don't carry image content in this Rust type system
        _ => false,
    })
}

// ---------------------------------------------------------------------------
// Dynamic header builder
// ---------------------------------------------------------------------------

/// Build the dynamic Copilot HTTP headers for a request.
///
/// Always sets:
/// - `X-Initiator`: `"user"` or `"agent"` depending on the last message role
/// - `Openai-Intent`: `"conversation-edits"`
///
/// Conditionally sets:
/// - `Copilot-Vision-Request`: `"true"` when `has_images` is true
pub fn build_copilot_dynamic_headers(messages: &[LlmMessage], has_images: bool) -> Vec<(String, String)> {
    let mut headers = vec![
        ("X-Initiator".to_string(), infer_copilot_initiator(messages).to_string()),
        ("Openai-Intent".to_string(), "conversation-edits".to_string()),
    ];

    if has_images {
        headers.push(("Copilot-Vision-Request".to_string(), "true".to_string()));
    }

    headers
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{LlmContent, LlmMessage};

    #[test]
    fn test_infer_initiator_last_user() {
        let messages = vec![LlmMessage::User {
            content: vec![LlmContent::Text("hi".into())],
        }];
        assert_eq!(infer_copilot_initiator(&messages), "user");
    }

    #[test]
    fn test_infer_initiator_last_assistant() {
        let messages = vec![LlmMessage::Assistant {
            content: "response".into(),
            tool_calls: vec![],
            thinking_blocks: vec![],
        }];
        assert_eq!(infer_copilot_initiator(&messages), "agent");
    }

    #[test]
    fn test_infer_initiator_last_tool() {
        let messages = vec![LlmMessage::Tool {
            tool_call_id: "tc1".into(),
            content: "result".into(),
            tool_name: None,
        }];
        assert_eq!(infer_copilot_initiator(&messages), "agent");
    }

    #[test]
    fn test_infer_initiator_empty() {
        assert_eq!(infer_copilot_initiator(&[]), "user");
    }

    #[test]
    fn test_has_vision_input_with_image() {
        let messages = vec![LlmMessage::User {
            content: vec![
                LlmContent::Text("look at this".into()),
                LlmContent::Image {
                    url: "data:image/png;base64,abc".into(),
                },
            ],
        }];
        assert!(has_copilot_vision_input(&messages));
    }

    #[test]
    fn test_has_vision_input_no_image() {
        let messages = vec![LlmMessage::User {
            content: vec![LlmContent::Text("just text".into())],
        }];
        assert!(!has_copilot_vision_input(&messages));
    }

    #[test]
    fn test_build_copilot_headers_no_images() {
        let messages = vec![LlmMessage::User {
            content: vec![LlmContent::Text("hi".into())],
        }];
        let headers = build_copilot_dynamic_headers(&messages, false);

        let header_map: std::collections::HashMap<&str, &str> =
            headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

        assert_eq!(header_map.get("X-Initiator"), Some(&"user"));
        assert_eq!(header_map.get("Openai-Intent"), Some(&"conversation-edits"));
        assert!(header_map.get("Copilot-Vision-Request").is_none());
    }

    #[test]
    fn test_build_copilot_headers_with_images() {
        let messages = vec![LlmMessage::User {
            content: vec![LlmContent::Image {
                url: "data:image/png;base64,abc".into(),
            }],
        }];
        let headers = build_copilot_dynamic_headers(&messages, true);

        let header_map: std::collections::HashMap<&str, &str> =
            headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

        assert_eq!(header_map.get("Copilot-Vision-Request"), Some(&"true"));
    }
}
