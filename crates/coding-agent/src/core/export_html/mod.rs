//! HTML export utilities for coding-agent sessions.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/export-html/`.
//!
//! Provides ANSI-to-HTML conversion (`ansi_to_html`) and a tool renderer
//! abstraction (`ToolHtmlRenderer`) used when exporting sessions to HTML.

pub mod ansi_to_html;
pub mod tool_renderer;
