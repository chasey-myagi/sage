//! Tool HTML renderer for custom tools in HTML export.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/export-html/tool-renderer.ts`.
//!
//! In TypeScript the renderer invokes tool TUI components and converts their
//! ANSI output to HTML.  This Rust translation provides the equivalent data
//! structures and a renderer builder — the actual TUI component invocation is
//! left as a trait object hook so the caller can plug in a real implementation
//! when the TUI layer is available.

use super::ansi_to_html::ansi_lines_to_html;

// ============================================================================
// Types
// ============================================================================

/// A single piece of tool result content.
#[derive(Debug, Clone)]
pub struct ToolResultContent {
    pub content_type: String,
    pub text: Option<String>,
    pub data: Option<String>,
    pub mime_type: Option<String>,
}

/// Rendered HTML for a tool result — collapsed and/or expanded views.
#[derive(Debug, Clone, Default)]
pub struct RenderedToolResult {
    pub collapsed: Option<String>,
    pub expanded: Option<String>,
}

/// Trait that abstracts the rendering of a single tool call / result to ANSI
/// lines.  Implement this to connect a real TUI component renderer.
///
/// Mirrors the `ToolDefinition.renderCall` / `renderResult` contract from
/// pi-mono.
pub trait ToolRenderer: Send + Sync {
    /// Render a tool call to ANSI text lines.
    ///
    /// Returns `None` if this renderer does not handle `tool_name`.
    fn render_call(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<Vec<String>>;

    /// Render a tool result to ANSI text lines (collapsed and/or expanded).
    ///
    /// Returns `None` if this renderer does not handle `tool_name`.
    fn render_result(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        result: &[ToolResultContent],
        details: &serde_json::Value,
        is_error: bool,
    ) -> Option<RenderedToolResult>;
}

// ============================================================================
// ToolHtmlRenderer
// ============================================================================

/// Converts tool call/result ANSI output to HTML.
///
/// Mirrors pi-mono `ToolHtmlRenderer`.
pub struct ToolHtmlRenderer {
    renderer: Box<dyn ToolRenderer>,
}

impl ToolHtmlRenderer {
    /// Create a new `ToolHtmlRenderer` backed by `renderer`.
    pub fn new(renderer: Box<dyn ToolRenderer>) -> Self {
        Self { renderer }
    }

    /// Render a tool call to HTML.
    ///
    /// Returns `None` if the underlying renderer does not handle `tool_name`.
    pub fn render_call(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<String> {
        let lines = self.renderer.render_call(tool_call_id, tool_name, args)?;
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        Some(ansi_lines_to_html(&refs))
    }

    /// Render a tool result to HTML.
    ///
    /// Returns `None` if the underlying renderer does not handle `tool_name`.
    pub fn render_result(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        result: &[ToolResultContent],
        details: &serde_json::Value,
        is_error: bool,
    ) -> Option<RenderedToolResult> {
        let rendered =
            self.renderer
                .render_result(tool_call_id, tool_name, result, details, is_error)?;

        let collapsed = rendered.collapsed.map(|lines| {
            // If lines is already HTML (from ansi_lines_to_html) pass through;
            // otherwise treat as raw ANSI.
            lines
        });
        let expanded = rendered.expanded.map(|lines| lines);

        Some(RenderedToolResult { collapsed, expanded })
    }
}

// ============================================================================
// NoopToolRenderer
// ============================================================================

/// A `ToolRenderer` that returns `None` for every tool.
///
/// Useful as a default/placeholder when no TUI rendering is available.
pub struct NoopToolRenderer;

impl ToolRenderer for NoopToolRenderer {
    fn render_call(
        &self,
        _tool_call_id: &str,
        _tool_name: &str,
        _args: &serde_json::Value,
    ) -> Option<Vec<String>> {
        None
    }

    fn render_result(
        &self,
        _tool_call_id: &str,
        _tool_name: &str,
        _result: &[ToolResultContent],
        _details: &serde_json::Value,
        _is_error: bool,
    ) -> Option<RenderedToolResult> {
        None
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeRenderer;

    impl ToolRenderer for FakeRenderer {
        fn render_call(
            &self,
            _id: &str,
            name: &str,
            _args: &serde_json::Value,
        ) -> Option<Vec<String>> {
            if name == "known" {
                Some(vec!["line one".to_string(), "line two".to_string()])
            } else {
                None
            }
        }

        fn render_result(
            &self,
            _id: &str,
            name: &str,
            _result: &[ToolResultContent],
            _details: &serde_json::Value,
            is_error: bool,
        ) -> Option<RenderedToolResult> {
            if name == "known" {
                Some(RenderedToolResult {
                    collapsed: Some(if is_error { "error".to_string() } else { "ok".to_string() }),
                    expanded: Some("expanded".to_string()),
                })
            } else {
                None
            }
        }
    }

    #[test]
    fn render_call_known_tool() {
        let hr = ToolHtmlRenderer::new(Box::new(FakeRenderer));
        let html = hr.render_call("id1", "known", &serde_json::json!({}));
        assert!(html.is_some());
        let html = html.unwrap();
        assert!(html.contains("<div class=\"ansi-line\">line one</div>"));
        assert!(html.contains("<div class=\"ansi-line\">line two</div>"));
    }

    #[test]
    fn render_call_unknown_tool_returns_none() {
        let hr = ToolHtmlRenderer::new(Box::new(FakeRenderer));
        let result = hr.render_call("id1", "unknown", &serde_json::json!({}));
        assert!(result.is_none());
    }

    #[test]
    fn render_result_known_tool() {
        let hr = ToolHtmlRenderer::new(Box::new(FakeRenderer));
        let result = hr.render_result("id1", "known", &[], &serde_json::json!({}), false);
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.collapsed.as_deref(), Some("ok"));
        assert_eq!(r.expanded.as_deref(), Some("expanded"));
    }

    #[test]
    fn render_result_error_flag() {
        let hr = ToolHtmlRenderer::new(Box::new(FakeRenderer));
        let result = hr.render_result("id1", "known", &[], &serde_json::json!({}), true);
        assert_eq!(result.unwrap().collapsed.as_deref(), Some("error"));
    }

    #[test]
    fn render_result_unknown_tool_returns_none() {
        let hr = ToolHtmlRenderer::new(Box::new(FakeRenderer));
        let result = hr.render_result("id1", "unknown", &[], &serde_json::json!({}), false);
        assert!(result.is_none());
    }

    #[test]
    fn noop_renderer_always_returns_none() {
        let hr = ToolHtmlRenderer::new(Box::new(NoopToolRenderer));
        assert!(hr.render_call("id", "bash", &serde_json::json!({})).is_none());
        assert!(hr.render_result("id", "bash", &[], &serde_json::json!({}), false).is_none());
    }
}
