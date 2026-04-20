//! Tool execution component.
//!
//! Translated from `components/tool-execution.ts`.
//!
//! Renders a tool call's invocation and result in a styled box.

use tui::tui::{Component, Container};
use tui::components::spacer::Spacer;
use tui::components::text::Text;

use crate::modes::interactive::theme::{get_theme, ThemeColor, ThemeBg};
use crate::modes::interactive::components::dynamic_border::DynamicBorder;
use crate::modes::interactive::components::keybinding_hints::key_hint;

// ============================================================================
// Tool result types
// ============================================================================

/// A content item in a tool result.
#[derive(Debug, Clone)]
pub enum ToolResultContent {
    Text { text: String },
    Image { data: String, mime_type: String },
}

/// Status of a tool execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    Success,
    Error,
}

// ============================================================================
// ToolExecutionComponent
// ============================================================================

/// Renders a tool execution (call + result) with expand/collapse.
pub struct ToolExecutionComponent {
    tool_name: String,
    tool_call_id: String,
    args: serde_json::Value,
    status: ToolStatus,
    result: Option<Vec<ToolResultContent>>,
    expanded: bool,
    is_partial: bool,
}

impl ToolExecutionComponent {
    pub fn new(
        tool_name: impl Into<String>,
        tool_call_id: impl Into<String>,
        args: serde_json::Value,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            tool_call_id: tool_call_id.into(),
            args,
            status: ToolStatus::Pending,
            result: None,
            expanded: false,
            is_partial: true,
        }
    }

    /// Mark the tool call as complete (args fully received).
    pub fn set_args_complete(&mut self) {
        self.is_partial = false;
    }

    /// Set the result of the tool execution.
    pub fn set_result(&mut self, content: Vec<ToolResultContent>, is_error: bool) {
        self.result = Some(content);
        self.status = if is_error { ToolStatus::Error } else { ToolStatus::Success };
        self.is_partial = false;
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
    }

    pub fn is_pending(&self) -> bool {
        self.status == ToolStatus::Pending
    }

    fn bg_color_key(&self) -> ThemeBg {
        match self.status {
            ToolStatus::Pending => ThemeBg::ToolPendingBg,
            ToolStatus::Success => ThemeBg::ToolSuccessBg,
            ToolStatus::Error => ThemeBg::ToolErrorBg,
        }
    }

    fn status_label(&self) -> &'static str {
        match self.status {
            ToolStatus::Pending => "…",
            ToolStatus::Success => "✓",
            ToolStatus::Error => "✗",
        }
    }

    fn format_args(&self) -> String {
        // Try to produce a concise one-line arg summary
        match &self.args {
            serde_json::Value::Object(map) => {
                let pairs: Vec<String> = map
                    .iter()
                    .take(3)
                    .map(|(k, v)| {
                        let val = match v {
                            serde_json::Value::String(s) => {
                                if s.len() > 40 {
                                    format!("\"{}…\"", &s[..37])
                                } else {
                                    format!("\"{}\"", s)
                                }
                            }
                            other => {
                                let s = other.to_string();
                                if s.len() > 40 { format!("{}…", &s[..37]) } else { s }
                            }
                        };
                        format!("{k}={val}")
                    })
                    .collect();
                if map.len() > 3 {
                    format!("{} +{} more", pairs.join(", "), map.len() - 3)
                } else {
                    pairs.join(", ")
                }
            }
            other => {
                let s = other.to_string();
                if s.len() > 60 { format!("{}…", &s[..57]) } else { s }
            }
        }
    }

    fn render_result_text(&self) -> Vec<String> {
        let Some(content) = &self.result else { return vec![] };
        content
            .iter()
            .filter_map(|c| match c {
                ToolResultContent::Text { text } => Some(text.clone()),
                ToolResultContent::Image { .. } => Some("[image]".to_string()),
            })
            .collect()
    }
}

impl Component for ToolExecutionComponent {
    fn render(&self, width: u16) -> Vec<String> {
        let t = get_theme();
        let mut container = Container::new();

        container.add_child(Box::new(Spacer::new(1)));

        let status = self.status_label();
        let args_summary = self.format_args();
        let title_line = format!(
            "{} {} {}",
            t.fg(ThemeColor::ToolTitle, status),
            t.fg(ThemeColor::Accent, &self.tool_name),
            t.fg(ThemeColor::Dim, &args_summary)
        );

        // Box with colored background
        let bg_key = self.bg_color_key();
        let mut box_content = Container::new();
        box_content.add_child(Box::new(Text::new(title_line, 1, 0)));

        if self.expanded {
            if let Some(result_lines) = Some(self.render_result_text()) {
                if !result_lines.is_empty() {
                    box_content.add_child(Box::new(Spacer::new(1)));
                    for line in result_lines {
                        let colored = t.fg(ThemeColor::ToolOutput, &line);
                        box_content.add_child(Box::new(Text::new(colored, 1, 0)));
                    }
                }
            }

            // Collapse hint
            box_content.add_child(Box::new(Spacer::new(1)));
            box_content.add_child(Box::new(Text::new(
                key_hint("ctrl+e", "collapse"),
                1,
                0,
            )));
        } else if self.result.is_some() {
            // Expand hint
            box_content.add_child(Box::new(Text::new(
                t.fg(ThemeColor::Dim, &format!("  {}", key_hint("ctrl+e", "expand"))),
                0,
                0,
            )));
        }

        container.add_child(Box::new(box_content));
        container.render(width)
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_tool_renders_name() {
        let comp = ToolExecutionComponent::new(
            "bash",
            "call-123",
            serde_json::json!({"command": "ls -la"}),
        );
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("bash"), "Expected 'bash' in: {text:?}");
    }

    #[test]
    fn completed_tool_shows_success_marker() {
        let mut comp = ToolExecutionComponent::new(
            "read_file",
            "call-456",
            serde_json::json!({"path": "/tmp/test.txt"}),
        );
        comp.set_result(
            vec![ToolResultContent::Text { text: "file contents".to_string() }],
            false,
        );
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("✓"), "Expected success marker in: {text:?}");
    }

    #[test]
    fn error_tool_shows_error_marker() {
        let mut comp = ToolExecutionComponent::new(
            "write_file",
            "call-789",
            serde_json::json!({}),
        );
        comp.set_result(
            vec![ToolResultContent::Text { text: "Permission denied".to_string() }],
            true,
        );
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("✗"), "Expected error marker in: {text:?}");
    }
}
