//! Assistant message component.
//!
//! Translated from `components/assistant-message.ts`.
//!
//! Renders an assistant turn with optional thinking blocks.

use tui::components::markdown::{DefaultTextStyle, Markdown, MarkdownTheme};
use tui::components::spacer::Spacer;
use tui::components::text::Text;
use tui::tui::{Component, Container};

use crate::modes::interactive::theme::{ThemeColor, get_theme};

// ============================================================================
// Message types (simplified from pi-ai)
// ============================================================================

/// Content block type within an assistant message.
#[derive(Debug, Clone)]
pub enum ContentBlock {
    /// Normal text response.
    Text { text: String },
    /// Extended thinking / reasoning.
    Thinking { thinking: String },
    /// Tool invocation (rendered separately by ToolExecutionComponent).
    ToolCall { id: String },
}

/// Stop reason for an assistant message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Aborted,
    Error,
}

/// A completed or streaming assistant message.
#[derive(Debug, Clone)]
pub struct AssistantMessage {
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<StopReason>,
    pub error_message: Option<String>,
}

// ============================================================================
// AssistantMessageComponent
// ============================================================================

/// Component that renders a complete assistant message.
///
/// Translated from `AssistantMessageComponent extends Container`.
pub struct AssistantMessageComponent {
    content_container: Container,
    hide_thinking_block: bool,
    last_message: Option<AssistantMessage>,
}

impl AssistantMessageComponent {
    pub fn new(message: Option<AssistantMessage>, hide_thinking_block: bool) -> Self {
        let mut comp = Self {
            content_container: Container::new(),
            hide_thinking_block,
            last_message: None,
        };
        if let Some(msg) = message {
            comp.update_content(msg);
        }
        comp
    }

    pub fn set_hide_thinking_block(&mut self, hide: bool) {
        self.hide_thinking_block = hide;
    }

    /// Update (or re-render) content from a new assistant message.
    pub fn update_content(&mut self, message: AssistantMessage) {
        self.last_message = Some(message.clone());
        self.content_container.clear();

        let has_visible_content = message.content.iter().any(|c| match c {
            ContentBlock::Text { text } => !text.trim().is_empty(),
            ContentBlock::Thinking { thinking } => !thinking.trim().is_empty(),
            _ => false,
        });

        if has_visible_content {
            self.content_container.add_child(Box::new(Spacer::new(1)));
        }

        let _md_theme = build_default_md_theme();
        let t = get_theme();

        for (i, content) in message.content.iter().enumerate() {
            match content {
                ContentBlock::Text { text } if !text.trim().is_empty() => {
                    self.content_container.add_child(Box::new(Markdown::new(
                        text.trim().to_string(),
                        1,
                        0,
                        build_default_md_theme(),
                        None,
                    )));
                }
                ContentBlock::Thinking { thinking } if !thinking.trim().is_empty() => {
                    // Check if there's visible content after this block
                    let has_visible_after = message.content[i + 1..].iter().any(|c| match c {
                        ContentBlock::Text { text } => !text.trim().is_empty(),
                        ContentBlock::Thinking { thinking } => !thinking.trim().is_empty(),
                        _ => false,
                    });

                    if self.hide_thinking_block {
                        let t2 = get_theme();
                        let label = t2.italic(&t2.fg(ThemeColor::ThinkingText, "Thinking..."));
                        self.content_container
                            .add_child(Box::new(Text::new(label, 1, 0)));
                        if has_visible_after {
                            self.content_container.add_child(Box::new(Spacer::new(1)));
                        }
                    } else {
                        let thinking_style = DefaultTextStyle {
                            color: Some(Box::new({
                                let t2 = t.clone();
                                move |s: &str| t2.fg(ThemeColor::ThinkingText, s)
                            })),
                            bg_color: None,
                            bold: false,
                            italic: true,
                            strikethrough: false,
                            underline: false,
                        };
                        self.content_container.add_child(Box::new(Markdown::new(
                            thinking.trim().to_string(),
                            1,
                            0,
                            build_default_md_theme(),
                            Some(thinking_style),
                        )));
                        if has_visible_after {
                            self.content_container.add_child(Box::new(Spacer::new(1)));
                        }
                    }
                }
                _ => {}
            }
        }

        // Show abort/error at the end if no tool calls
        let has_tool_calls = message
            .content
            .iter()
            .any(|c| matches!(c, ContentBlock::ToolCall { .. }));
        if !has_tool_calls {
            match message.stop_reason.as_ref() {
                Some(StopReason::Aborted) => {
                    let msg = message
                        .error_message
                        .as_deref()
                        .filter(|m| *m != "Request was aborted")
                        .unwrap_or("Operation aborted");
                    let t2 = get_theme();
                    let err_text = t2.fg(ThemeColor::Error, msg);
                    self.content_container.add_child(Box::new(Spacer::new(1)));
                    self.content_container
                        .add_child(Box::new(Text::new(err_text, 1, 0)));
                }
                Some(StopReason::Error) => {
                    let t2 = get_theme();
                    let msg = message.error_message.as_deref().unwrap_or("Unknown error");
                    let err_text = t2.fg(ThemeColor::Error, &format!("Error: {msg}"));
                    self.content_container.add_child(Box::new(Spacer::new(1)));
                    self.content_container
                        .add_child(Box::new(Text::new(err_text, 1, 0)));
                }
                _ => {}
            }
        }
    }
}

fn build_default_md_theme() -> MarkdownTheme {
    let t = get_theme();
    MarkdownTheme {
        heading: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdHeading, s)
        }),
        link: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdLink, s)
        }),
        link_url: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdLinkUrl, s)
        }),
        code: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdCode, s)
        }),
        code_block: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdCodeBlock, s)
        }),
        code_block_border: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdCodeBlockBorder, s)
        }),
        quote: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdQuote, s)
        }),
        quote_border: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdQuoteBorder, s)
        }),
        hr: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdHr, s)
        }),
        list_bullet: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdListBullet, s)
        }),
        bold: Box::new(|s: &str| format!("\x1b[1m{s}\x1b[22m")),
        italic: Box::new(|s: &str| format!("\x1b[3m{s}\x1b[23m")),
        strikethrough: Box::new(|s: &str| format!("\x1b[9m{s}\x1b[29m")),
        underline: Box::new(|s: &str| format!("\x1b[4m{s}\x1b[24m")),
        code_block_indent: None,
        highlight_code: None,
    }
}

impl Component for AssistantMessageComponent {
    fn render(&self, width: u16) -> Vec<String> {
        self.content_container.render(width)
    }

    fn invalidate(&mut self) {
        self.content_container.invalidate();
        if let Some(msg) = self.last_message.clone() {
            self.update_content(msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_message_renders_nothing() {
        let msg = AssistantMessage {
            content: vec![],
            stop_reason: None,
            error_message: None,
        };
        let comp = AssistantMessageComponent::new(Some(msg), false);
        let lines = comp.render(80);
        // No visible content = no spacer
        assert!(lines.is_empty());
    }

    #[test]
    fn text_message_renders() {
        let msg = AssistantMessage {
            content: vec![ContentBlock::Text {
                text: "Hello world".to_string(),
            }],
            stop_reason: None,
            error_message: None,
        };
        let comp = AssistantMessageComponent::new(Some(msg), false);
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(
            text.contains("Hello world"),
            "Expected 'Hello world' in: {text:?}"
        );
    }

    #[test]
    fn thinking_hidden_shows_label() {
        let msg = AssistantMessage {
            content: vec![ContentBlock::Thinking {
                thinking: "deep thought".to_string(),
            }],
            stop_reason: None,
            error_message: None,
        };
        let comp = AssistantMessageComponent::new(Some(msg), true);
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(
            text.contains("Thinking..."),
            "Expected 'Thinking...' in: {text:?}"
        );
    }

    #[test]
    fn error_stop_reason_shows_error() {
        let msg = AssistantMessage {
            content: vec![ContentBlock::Text {
                text: "partial".to_string(),
            }],
            stop_reason: Some(StopReason::Error),
            error_message: Some("Something broke".to_string()),
        };
        let comp = AssistantMessageComponent::new(Some(msg), false);
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("Error:"), "Expected 'Error:' in: {text:?}");
    }
}
