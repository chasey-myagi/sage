//! Interactive TUI mode for the coding agent.
//!
//! Translated from pi-mono `packages/coding-agent/src/modes/interactive/interactive-mode.ts`.
//!
//! This is the full-screen TUI mode launched when `sage` is invoked without
//! `--print` / `--mode json|rpc`. The implementation here is a structural
//! skeleton; the ratatui rendering details are fleshed out in the `tui` crate.

pub mod approval;
pub mod components;
pub mod theme;

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, OnceLock};

use regex::Regex;

use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyModifiers,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt as _;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use tokio::sync::mpsc;

use unicode_width::UnicodeWidthStr as _;

use crate::agent_session::AgentDelta;
use crate::core::slash_commands::BUILTIN_SLASH_COMMANDS;
use crate::modes::interactive::approval::{ApprovalRequest, ApprovalResponse};
use crate::modes::interactive::components::diff::render_diff_ratatui;
use crate::modes::interactive::theme::{Theme, ThemeBg, ThemeColor, get_theme};

// ============================================================================
// InteractiveMode
// ============================================================================

/// Options passed to [`InteractiveMode`].
#[derive(Debug, Default)]
pub struct InteractiveModeOptions {
    /// Optional initial message to send on start.
    pub initial_message: Option<String>,
    /// Whether to show verbose startup output.
    pub verbose: bool,
    /// Optional model-fallback warning message.
    pub model_fallback_message: Option<String>,
    /// Optional provider-migration notices.
    pub migrated_providers: Vec<String>,
}

/// State held by the interactive TUI loop.
pub struct InteractiveMode {
    options: InteractiveModeOptions,
    input_buffer: String,
    messages: Vec<ChatMessage>,
    running: bool,
    agent_rx: Option<mpsc::UnboundedReceiver<AgentDelta>>,
    is_thinking: bool,
    provider_id: Option<String>,
    model_id: Option<String>,
    session_input_tokens: u64,
    session_output_tokens: u64,
    session_cost_usd: f64,
    /// Current scroll offset in display lines.
    scroll_top: u16,
    /// When true, scroll_top tracks the bottom of content as it grows.
    is_sticky: bool,
    /// Viewport height cached from last render (lines, no border).
    last_viewport_height: u16,
    /// Terminal width cached from last render (columns).
    last_terminal_width: u16,
    /// Tick counter for spinner animation (incremented every ~50 ms).
    tick: u64,
    // ── Approval channel ──────────────────────────────────────────────────
    approval_tx: mpsc::UnboundedSender<ApprovalRequest>,
    approval_rx: mpsc::UnboundedReceiver<ApprovalRequest>,
    pending_approval: Option<ApprovalRequest>,
    /// Per-session tool rules: true=AllowAlways, false=DenyAlways.
    session_rules: Arc<Mutex<HashMap<String, bool>>>,
    // ── Autocomplete (slash commands + @ file) ────────────────────────────
    completion_matches: Vec<(String, String)>, // (primary, hint)
    completion_selected: usize,
    // ── Agent task handle ─────────────────────────────────────────────────
    /// Handle to the running agent task; aborted before spawning a new one.
    agent_handle: Option<tokio::task::JoinHandle<()>>,
}

/// A single chat turn in the history display.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

/// Speaker for a chat message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    /// A tool invocation — pending while `pending == true`.
    Tool {
        name: String,
        pending: bool,
        success: bool,
    },
    /// A fatal agent error shown inline.
    Error,
}

impl InteractiveMode {
    /// Create a new InteractiveMode with the given options.
    pub fn new(options: InteractiveModeOptions) -> Self {
        let (approval_tx, approval_rx) = mpsc::unbounded_channel();
        Self {
            options,
            input_buffer: String::new(),
            messages: Vec::new(),
            running: false,
            agent_rx: None,
            is_thinking: false,
            provider_id: None,
            model_id: None,
            session_input_tokens: 0,
            session_output_tokens: 0,
            session_cost_usd: 0.0,
            scroll_top: 0,
            is_sticky: true,
            last_viewport_height: 0,
            last_terminal_width: 80,
            tick: 0,
            approval_tx,
            approval_rx,
            pending_approval: None,
            session_rules: Arc::new(Mutex::new(HashMap::new())),
            completion_matches: Vec::new(),
            completion_selected: 0,
            agent_handle: None,
        }
    }

    pub fn set_provider(&mut self, provider: Option<String>) {
        self.provider_id = provider;
    }

    pub fn set_model(&mut self, model: Option<String>) {
        self.model_id = model;
    }

    /// Initialise the TUI (equivalent to `interactiveMode.init()` in TS).
    /// Sets up the terminal and renders the initial frame.
    pub async fn init(&mut self) -> anyhow::Result<()> {
        if let Some(msg) = &self.options.initial_message.clone() {
            self.messages.push(ChatMessage {
                role: MessageRole::User,
                content: msg.clone(),
            });
        }
        Ok(())
    }

    /// Run the interactive TUI event loop (equivalent to `interactiveMode.run()`).
    pub async fn run(&mut self) -> anyhow::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        self.running = true;

        // Send initial message to agent if provided
        if let Some(msg) = self.options.initial_message.clone()
            && !msg.is_empty()
        {
            self.messages.push(ChatMessage {
                role: MessageRole::User,
                content: msg.clone(),
            });
            self.messages.push(ChatMessage {
                role: MessageRole::Assistant,
                content: String::new(),
            });
            self.spawn_agent(msg);
        }

        let mut event_stream = EventStream::new();

        loop {
            // Drain one approval request from agent tasks (show dialog; handle one at a time).
            // Guard: only accept a new request when none is pending, to prevent overwriting
            // a live request (which would silently deny it when the sender is dropped).
            if self.pending_approval.is_none() {
                if let Ok(req) = self.approval_rx.try_recv() {
                    self.pending_approval = Some(req);
                }
            }

            // Drain agent response deltas (disjoint field borrows allow updates inside)
            let mut disconnected = false;
            if let Some(rx) = &mut self.agent_rx {
                loop {
                    match rx.try_recv() {
                        Ok(AgentDelta::Text(delta)) => {
                            if let Some(last) = self.messages.last_mut() {
                                if last.role == MessageRole::Assistant {
                                    last.content.push_str(&delta);
                                } else {
                                    self.messages.push(ChatMessage {
                                        role: MessageRole::Assistant,
                                        content: delta,
                                    });
                                }
                            }
                        }
                        Ok(AgentDelta::TurnUsage {
                            usage,
                            model,
                            is_fast,
                        }) => {
                            self.session_input_tokens += usage.input;
                            self.session_output_tokens += usage.output;
                            let cost =
                                ai::model_pricing::calculate_usd_cost(&usage, &model, is_fast);
                            self.session_cost_usd += cost.total;
                        }
                        Ok(AgentDelta::ToolStart { name, args_preview }) => {
                            let content = if args_preview.is_empty() {
                                name.clone()
                            } else {
                                format!("{name}({args_preview})")
                            };
                            self.messages.push(ChatMessage {
                                role: MessageRole::Tool {
                                    name,
                                    pending: true,
                                    success: false,
                                },
                                content,
                            });
                        }
                        Ok(AgentDelta::ToolEnd {
                            name,
                            success,
                            output_preview,
                        }) => {
                            let pos = self.messages.iter().rposition(|m| {
                                matches!(&m.role, MessageRole::Tool { name: n, pending: true, .. } if n == &name)
                            });
                            if let Some(idx) = pos {
                                if !output_preview.is_empty() {
                                    let existing = self.messages[idx].content.clone();
                                    self.messages[idx].content = format!(
                                        "{existing} · {}",
                                        output_preview.chars().take(80).collect::<String>()
                                    );
                                }
                                self.messages[idx].role = MessageRole::Tool {
                                    name,
                                    pending: false,
                                    success,
                                };
                            }
                        }
                        Ok(AgentDelta::Error(err)) => {
                            self.is_thinking = false;
                            self.messages.push(ChatMessage {
                                role: MessageRole::Error,
                                content: err,
                            });
                        }
                        Err(mpsc::error::TryRecvError::Empty) => break,
                        Err(mpsc::error::TryRecvError::Disconnected) => {
                            disconnected = true;
                            break;
                        }
                    }
                }
            }
            if disconnected {
                self.agent_rx = None;
                self.agent_handle = None;
                self.is_thinking = false;
            }

            // Sticky scroll: pin to bottom as new content arrives.
            // Must be computed before terminal.draw() so render() is side-effect-free.
            if self.is_sticky {
                let total = Self::total_content_lines(&self.messages, self.last_terminal_width);
                self.scroll_top = total.saturating_sub(self.last_viewport_height);
            }
            terminal.draw(|f| self.render(f))?;

            tokio::select! {
                maybe_event = event_stream.next() => {
                    let Some(Ok(event)) = maybe_event else { continue };
                    match event {
                        Event::Key(key) => {
                            // Approval dialog intercepts most keys when active.
                            if self.pending_approval.is_some() {
                                match (key.code, key.modifiers) {
                                    // Ctrl+C / Ctrl+D always exits, even from the dialog.
                                    (KeyCode::Char('c'), KeyModifiers::CONTROL)
                                    | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                                        self.abort_agent();
                                        self.running = false;
                                        break;
                                    }
                                    (KeyCode::Char('y'), _) => {
                                        self.resolve_approval(ApprovalResponse::Allow);
                                    }
                                    (KeyCode::Char('Y'), _) => {
                                        self.resolve_approval(ApprovalResponse::AllowAlways);
                                    }
                                    (KeyCode::Char('n'), _) | (KeyCode::Esc, _) => {
                                        self.resolve_approval(ApprovalResponse::Deny);
                                    }
                                    (KeyCode::Char('N'), _) => {
                                        self.resolve_approval(ApprovalResponse::DenyAlways);
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            match (key.code, key.modifiers) {
                                (KeyCode::Char('c'), KeyModifiers::CONTROL)
                                | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                                    self.abort_agent();
                                    self.running = false;
                                    break;
                                }
                                // Scroll shortcuts — active only when input buffer is empty
                                // so j/k/g/G are available for typing when composing a message.
                                (KeyCode::Char('j'), KeyModifiers::NONE)
                                    if self.input_buffer.is_empty() =>
                                {
                                    self.is_sticky = false;
                                    self.scroll_top = self.scroll_top.saturating_add(1);
                                    self.clamp_scroll();
                                }
                                (KeyCode::Char('k'), KeyModifiers::NONE)
                                    if self.input_buffer.is_empty() =>
                                {
                                    self.is_sticky = false;
                                    self.scroll_top = self.scroll_top.saturating_sub(1);
                                }
                                (KeyCode::Char('g'), KeyModifiers::NONE)
                                    if self.input_buffer.is_empty() =>
                                {
                                    self.is_sticky = false;
                                    self.scroll_top = 0;
                                }
                                (KeyCode::Char('G'), modifiers)
                                    if self.input_buffer.is_empty()
                                        && (modifiers.is_empty()
                                            || modifiers == KeyModifiers::SHIFT) =>
                                {
                                    self.is_sticky = true;
                                }
                                // PageUp/PageDown always scroll regardless of input state
                                (KeyCode::PageDown, _) => {
                                    self.is_sticky = false;
                                    self.scroll_top = self
                                        .scroll_top
                                        .saturating_add(self.last_viewport_height);
                                    self.clamp_scroll();
                                }
                                (KeyCode::PageUp, _) => {
                                    self.is_sticky = false;
                                    self.scroll_top = self
                                        .scroll_top
                                        .saturating_sub(self.last_viewport_height);
                                }
                                // Slash command navigation
                                (KeyCode::Tab, _) | (KeyCode::Down, _)
                                    if !self.completion_matches.is_empty() =>
                                {
                                    let n = self.completion_matches.len();
                                    self.completion_selected = (self.completion_selected + 1) % n;
                                }
                                (KeyCode::Up, _) if !self.completion_matches.is_empty() => {
                                    let n = self.completion_matches.len();
                                    self.completion_selected = if self.completion_selected == 0 {
                                        n - 1
                                    } else {
                                        self.completion_selected - 1
                                    };
                                }
                                // Input handling
                                (KeyCode::Enter, _) if !self.is_thinking => {
                                    // Completion selection
                                    if !self.completion_matches.is_empty() {
                                        let selected = self.completion_selected
                                            .min(self.completion_matches.len() - 1);
                                        let chosen = self.completion_matches[selected].0.clone();
                                        if self.input_buffer.starts_with('/') {
                                            self.input_buffer = format!("/{chosen} ");
                                        } else if let Some(at_pos) = self.input_buffer.rfind('@') {
                                            self.input_buffer.truncate(at_pos + 1);
                                            self.input_buffer.push_str(&chosen);
                                            self.input_buffer.push(' ');
                                        }
                                        self.completion_matches.clear();
                                        self.completion_selected = 0;
                                        continue;
                                    }
                                    let input = std::mem::take(&mut self.input_buffer);
                                    self.completion_matches.clear();
                                    self.completion_selected = 0;
                                    if !input.trim().is_empty() {
                                        self.messages.push(ChatMessage {
                                            role: MessageRole::User,
                                            content: input.clone(),
                                        });
                                        self.messages.push(ChatMessage {
                                            role: MessageRole::Assistant,
                                            content: String::new(),
                                        });
                                        // Re-enable sticky so new response is always visible.
                                        self.is_sticky = true;
                                        self.spawn_agent(input);
                                    }
                                }
                                (KeyCode::Backspace, _) => {
                                    self.input_buffer.pop();
                                    self.update_completion_matches();
                                }
                                (KeyCode::Esc, _) if !self.completion_matches.is_empty() => {
                                    self.completion_matches.clear();
                                    self.completion_selected = 0;
                                }
                                (KeyCode::Char(c), _) => {
                                    self.input_buffer.push(c);
                                    self.update_completion_matches();
                                }
                                _ => {}
                            }
                        }
                        Event::Mouse(mouse) => match mouse.kind {
                            MouseEventKind::ScrollDown => {
                                self.is_sticky = false;
                                self.scroll_top = self.scroll_top.saturating_add(3);
                                self.clamp_scroll();
                            }
                            MouseEventKind::ScrollUp => {
                                self.is_sticky = false;
                                self.scroll_top = self.scroll_top.saturating_sub(3);
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                    self.tick = self.tick.wrapping_add(1);
                }
            }
        }

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;
        Ok(())
    }

    /// Abort any running agent task and deny its pending approval (if any).
    fn abort_agent(&mut self) {
        // Deny pending approval so the agent-side oneshot doesn't hang until timeout.
        if let Some(req) = self.pending_approval.take() {
            let _ = req.response_tx.send(ApprovalResponse::Deny);
        }
        if let Some(handle) = self.agent_handle.take() {
            handle.abort();
        }
        self.is_thinking = false;
    }

    fn spawn_agent(&mut self, message: String) {
        // Abort any prior orphan task before starting a new one.
        self.abort_agent();

        let (tx, rx) = mpsc::unbounded_channel::<AgentDelta>();
        self.agent_rx = Some(rx);
        self.is_thinking = true;

        let provider_id = self.provider_id.clone();
        let model_id = self.model_id.clone();
        let error_tx = tx.clone();
        let approval_tx = self.approval_tx.clone();
        let session_rules = Arc::clone(&self.session_rules);

        let handle = tokio::spawn(async move {
            if let Err(e) = crate::agent_session::run_agent_session_to_channel(
                message,
                model_id,
                provider_id,
                None,
                tx,
                "default".to_string(), // TODO(permission_mode): read from settings when implemented
                Some(approval_tx),
                session_rules,
            )
            .await
            {
                let _ = error_tx.send(AgentDelta::Error(e.to_string()));
            }
        });
        self.agent_handle = Some(handle);
    }

    /// Resolve the current pending approval and send response to agent.
    fn resolve_approval(&mut self, response: ApprovalResponse) {
        if let Some(req) = self.pending_approval.take() {
            let _ = req.response_tx.send(response);
        }
    }

    /// Update completion_matches based on current input_buffer.
    ///
    /// - `/prefix` → builtin slash commands starting with prefix
    /// - `…@prefix` → files matching prefix* in the working directory
    fn update_completion_matches(&mut self) {
        if self.input_buffer.starts_with('/') {
            let prefix = &self.input_buffer[1..];
            self.completion_matches = BUILTIN_SLASH_COMMANDS
                .iter()
                .filter(|c| c.name.starts_with(prefix))
                .map(|c| (c.name.to_string(), c.description.to_string()))
                .collect();
        } else if let Some(at_pos) = self.input_buffer.rfind('@') {
            // Only trigger on `@` that is preceded by whitespace (or at start).
            // `at_pos` from `rfind('@')` is always a valid char boundary (@ is ASCII).
            // `map_or(true, …)` handles the start-of-string case (empty slice → None → true).
            let preceded_by_space = self.input_buffer[..at_pos]
                .chars()
                .next_back()
                .map_or(true, |c| c.is_whitespace());
            if preceded_by_space {
                let file_prefix = &self.input_buffer[at_pos + 1..];
                self.completion_matches = Self::file_completions_for(file_prefix);
            } else {
                self.completion_matches.clear();
            }
        } else {
            self.completion_matches.clear();
        }
        // Always reset selection so the list doesn't "jump" when items change.
        self.completion_selected = 0;
    }

    /// List files/dirs that start with `prefix` (up to MAX_COMPLETIONS results).
    ///
    /// Escapes glob metacharacters in the prefix and restricts results to paths
    /// that are inside (or equal to) the current working directory.
    fn file_completions_for(prefix: &str) -> Vec<(String, String)> {
        const MAX_COMPLETIONS: usize = 5;
        let escaped = glob::Pattern::escape(prefix);
        let pattern = format!("{escaped}*");
        // Canonicalize cwd so that symlink paths (e.g. macOS /tmp → /private/tmp)
        // are resolved before prefix-checking canonicalized child paths.
        let cwd = std::env::current_dir().and_then(|c| c.canonicalize()).ok();
        match glob::glob(&pattern) {
            Ok(paths) => {
                let mut results: Vec<(String, String)> = paths
                    .filter_map(|e| e.ok())
                    .filter(|p| {
                        // Reject paths that escape the working directory.
                        if let Some(cwd) = &cwd {
                            p.canonicalize()
                                .map(|c| c.starts_with(cwd))
                                .unwrap_or(false)
                        } else {
                            true
                        }
                    })
                    .take(MAX_COMPLETIONS)
                    .map(|p| {
                        let hint = if p.is_dir() {
                            "dir".to_string()
                        } else {
                            "file".to_string()
                        };
                        (p.display().to_string(), hint)
                    })
                    .collect();
                results.sort_by(|a, b| a.0.cmp(&b.0));
                results
            }
            Err(_) => Vec::new(),
        }
    }

    /// Stop the interactive mode (e.g., for startup benchmarks).
    pub fn stop(&mut self) {
        self.running = false;
    }

    // ── Scroll helpers ──────────────────────────────────────────────────────

    /// Estimate display height of one message in terminal rows.
    ///
    /// Each content line is divided into chunks of `effective_width` columns.
    /// The prefix (`  ❯ ` or `  ◆ `, 4 cols) is part of the first line's budget.
    fn compute_message_height(msg: &ChatMessage, inner_width: u16) -> u16 {
        let first_line = msg.content.lines().next().unwrap_or("");
        if first_line.starts_with("diff --git") {
            // Diff branch renders: 1 prefix line + diff lines (each with 4-space indent).
            // Accumulate as usize to avoid u16 overflow on large diffs.
            let diff_rows: usize = msg
                .content
                .lines()
                .map(|l| {
                    let cols = l.width().saturating_add(4); // 4 = indent prefix
                    let w = (inner_width.max(1)) as usize;
                    cols.div_ceil(w).max(1)
                })
                .sum();
            return 1 + u16::try_from(diff_rows).unwrap_or(u16::MAX);
        }
        // prefix occupies the first ratatui row only; subsequent wrapped rows use full width.
        let prefix_len: u16 = 4; // "  ❯ " / "  ◆ " / etc. — all 4 display cols
        let first_capacity = inner_width.saturating_sub(prefix_len).max(1);
        let count: u16 = msg
            .content
            .lines()
            .map(|line| {
                let n = line.width() as u16;
                if n == 0 || n <= first_capacity {
                    1
                } else {
                    1 + (n - first_capacity).div_ceil(inner_width.max(1))
                }
            })
            .sum();
        count.max(1)
    }

    fn total_content_lines(messages: &[ChatMessage], inner_width: u16) -> u16 {
        messages
            .iter()
            .map(|m| Self::compute_message_height(m, inner_width))
            .sum()
    }

    /// Parse a text line into styled ratatui spans, handling inline Markdown:
    /// **bold**, *italic*, `code`, with list prefix substitution.
    fn parse_inline_markdown(text: &str, theme: &Theme) -> Vec<Span<'static>> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| {
            // Match: `code`, **bold**, *italic* (in precedence order)
            Regex::new(r"`([^`]+)`|\*\*([^*]+)\*\*|\*([^*\s][^*]*)\*").unwrap()
        });

        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut last = 0;

        for cap in re.captures_iter(text) {
            let m = cap.get(0).unwrap();
            if m.start() > last {
                spans.push(Span::raw(text[last..m.start()].to_string()));
            }
            if let Some(code) = cap.get(1) {
                let style = Style::default()
                    .fg(theme.ratatui_fg(ThemeColor::Muted))
                    .bg(theme.ratatui_bg(ThemeBg::CodeBg));
                spans.push(Span::styled(code.as_str().to_string(), style));
            } else if let Some(bold) = cap.get(2) {
                spans.push(Span::styled(
                    bold.as_str().to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            } else if let Some(italic) = cap.get(3) {
                spans.push(Span::styled(
                    italic.as_str().to_string(),
                    Style::default().add_modifier(Modifier::ITALIC),
                ));
            }
            last = m.end();
        }
        if last < text.len() {
            spans.push(Span::raw(text[last..].to_string()));
        }
        if spans.is_empty() {
            spans.push(Span::raw(text.to_string()));
        }
        spans
    }

    /// Convert a `ChatMessage` to ratatui `Line`s, handling embedded newlines.
    ///
    /// First content line: `  ❯ {text}` (user) or `  ◆ {text}` (assistant).
    /// Subsequent lines: indented by 4 spaces to align under the text.
    /// Diff content (detected by leading `diff --git` header) is rendered
    /// with ANSI color codes via `render_diff()`.
    fn message_to_lines(
        msg: &ChatMessage,
        theme: &Theme,
        terminal_width: u16,
    ) -> Vec<Line<'static>> {
        let (indicator, indicator_color, bg_color) = match &msg.role {
            MessageRole::User => (
                "❯",
                theme.ratatui_fg(ThemeColor::Accent),
                Some(theme.ratatui_bg(ThemeBg::UserMessageBg)),
            ),
            MessageRole::Assistant => ("◆", theme.ratatui_fg(ThemeColor::Accent), None),
            MessageRole::System => ("◆", theme.ratatui_fg(ThemeColor::Warning), None),
            MessageRole::Tool { pending: true, .. } => {
                ("⏺", theme.ratatui_fg(ThemeColor::Warning), None)
            }
            MessageRole::Tool { success: true, .. } => {
                ("✓", theme.ratatui_fg(ThemeColor::Muted), None)
            }
            MessageRole::Tool { .. } => ("✘", theme.ratatui_fg(ThemeColor::Error), None),
            MessageRole::Error => ("✘", theme.ratatui_fg(ThemeColor::Error), None),
        };

        let prefix = format!("  {indicator} ");
        let indent = "    ".to_string();
        let mut lines: Vec<Line<'static>> = Vec::new();

        // Detect unified diff content (requires the canonical `diff --git` header).
        let first_line = msg.content.lines().next().unwrap_or("");
        let looks_like_diff = first_line.starts_with("diff --git");

        if looks_like_diff && bg_color.is_none() {
            // Render prefix on its own line, then diff lines with ratatui-native colors
            // (no ANSI escape strings — ratatui uses Style/Span, not raw escape codes).
            lines.push(Line::from(Span::styled(
                prefix.clone(),
                Style::default().fg(indicator_color),
            )));
            for mut diff_line in render_diff_ratatui(&msg.content, theme) {
                // Prepend 4-space indent to each span in the line.
                diff_line.spans.insert(0, Span::raw("    "));
                lines.push(diff_line);
            }
            return lines;
        }

        let content_lines: Vec<&str> = if msg.content.is_empty() {
            vec![""]
        } else {
            msg.content.lines().collect()
        };

        for (i, text_line) in content_lines.iter().enumerate() {
            let lead_text = if i == 0 {
                prefix.clone()
            } else {
                indent.clone()
            };
            let lead = Span::styled(lead_text.clone(), Style::default().fg(indicator_color));
            let content_span = Span::styled(text_line.to_string(), Style::default());

            if let Some(bg) = bg_color {
                // Pad the line to terminal width for full-width background.
                let used = lead_text.width() + text_line.width();
                let pad = (terminal_width as usize).saturating_sub(used);
                let padding = Span::styled(" ".repeat(pad), Style::default().bg(bg));
                let lead_bg = Span::styled(lead_text, Style::default().fg(indicator_color).bg(bg));
                let content_bg = Span::styled(text_line.to_string(), Style::default().bg(bg));
                lines.push(Line::from(vec![lead_bg, content_bg, padding]));
            } else if matches!(msg.role, MessageRole::Assistant) {
                // Render inline Markdown for assistant messages.
                // Unordered list markers (`- ` / `* `) are replaced with a styled bullet.
                let (body, list_bullet) = if let Some(rest) = text_line
                    .strip_prefix("- ")
                    .or_else(|| text_line.strip_prefix("* "))
                {
                    let bullet = Span::styled(
                        "• ".to_string(),
                        Style::default().fg(theme.ratatui_fg(ThemeColor::MdListBullet)),
                    );
                    (rest, Some(bullet))
                } else {
                    (*text_line, None)
                };
                let mut line_spans = vec![lead];
                if let Some(b) = list_bullet {
                    line_spans.push(b);
                }
                line_spans.extend(Self::parse_inline_markdown(body, theme));
                lines.push(Line::from(line_spans));
            } else {
                lines.push(Line::from(vec![lead, content_span]));
            }
        }

        lines
    }

    /// Clamp `scroll_top` so we never scroll past the last line of content.
    fn clamp_scroll(&mut self) {
        let inner_width = self.last_terminal_width; // borderless — full width
        let total = Self::total_content_lines(&self.messages, inner_width);
        let max = total.saturating_sub(self.last_viewport_height);
        self.scroll_top = self.scroll_top.min(max);
    }

    // ── Format helpers ──────────────────────────────────────────────────────

    fn format_tokens(n: u64) -> String {
        if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1_000_000.0)
        } else if n >= 1_000 {
            format!("{:.1}k", n as f64 / 1_000.0)
        } else {
            n.to_string()
        }
    }

    fn format_cost(usd: f64) -> String {
        if usd >= 1.0 {
            format!("${:.2}", usd)
        } else {
            format!("${:.4}", usd)
        }
    }

    // ── Layout helpers ──────────────────────────────────────────────────────

    /// Compute a centered rect of fixed height inside `r`, as wide as `percent_x` percent.
    fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
        let vert = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(height),
                Constraint::Min(0),
            ])
            .split(r);

        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ])
            .split(vert[1])[1]
    }

    // ── Render ──────────────────────────────────────────────────────────────

    /// Render the TUI frame — CC-style borderless layout.
    fn render(&mut self, f: &mut ratatui::Frame) {
        let size = f.area();
        let width = size.width;
        let theme = get_theme();

        // Dynamic layout: add slash menu row if matches exist.
        let menu_rows = if self.completion_matches.is_empty() {
            0u16
        } else {
            (self.completion_matches.len().min(5) + 2) as u16
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),         // header
                Constraint::Min(0),            // messages
                Constraint::Length(menu_rows), // slash menu (0 if empty)
                Constraint::Length(1),         // divider
                Constraint::Length(1),         // input prompt
            ])
            .split(size);

        let viewport_height = chunks[1].height;

        // Cache dimensions used by key/mouse handlers between frames.
        self.last_terminal_width = width;
        self.last_viewport_height = viewport_height;

        // ── Header ────────────────────────────────────────────────────────
        let model_label = self.model_id.as_deref().unwrap_or("claude");
        let header_left = format!("  sage  {model_label}");
        let stats = format!(
            "↑{}  ↓{}  {}  ",
            Self::format_tokens(self.session_input_tokens),
            Self::format_tokens(self.session_output_tokens),
            Self::format_cost(self.session_cost_usd),
        );
        let left_len = header_left.width() as u16;
        let stats_len = stats.width() as u16;
        let gap = width.saturating_sub(left_len + stats_len);
        let header_line = Line::from(vec![
            Span::styled(
                "  sage  ".to_string(),
                Style::default()
                    .fg(theme.ratatui_fg(ThemeColor::Accent))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                model_label.to_string(),
                Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
            ),
            Span::raw(" ".repeat(gap as usize)),
            Span::styled(
                stats,
                Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
            ),
        ]);
        f.render_widget(Paragraph::new(vec![header_line]), chunks[0]);

        // ── Messages ──────────────────────────────────────────────────────
        let mut message_lines: Vec<Line> = self
            .messages
            .iter()
            .flat_map(|m| Self::message_to_lines(m, &theme, width))
            .collect();

        // Empty state: welcome prompt when no conversation has started.
        if message_lines.is_empty() && !self.is_thinking {
            message_lines = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "  ◆ ",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Accent)),
                    ),
                    Span::styled(
                        "What can I help you with?",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
                    ),
                ]),
            ];
        }

        let total_lines = Self::total_content_lines(&self.messages, width);
        let messages_widget = Paragraph::new(message_lines)
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_top, 0));
        f.render_widget(messages_widget, chunks[1]);

        // ── Completion menu (slash commands or @ files) ───────────────────
        let is_slash_mode = self.input_buffer.starts_with('/');
        if menu_rows > 0 {
            let visible: Vec<_> = self.completion_matches.iter().take(5).collect();
            let menu_lines: Vec<Line> = visible
                .iter()
                .enumerate()
                .map(|(i, (primary, hint))| {
                    let selected = i == self.completion_selected;
                    let bg = if selected {
                        theme.ratatui_bg(ThemeBg::SelectedBg)
                    } else {
                        Color::Reset
                    };
                    let label = if is_slash_mode {
                        format!("  /{primary:<12}")
                    } else {
                        format!("  {primary:<14}")
                    };
                    Line::from(vec![
                        Span::styled(
                            label,
                            Style::default()
                                .fg(theme.ratatui_fg(ThemeColor::Accent))
                                .bg(bg),
                        ),
                        Span::styled(
                            format!("  {hint}"),
                            Style::default()
                                .fg(theme.ratatui_fg(ThemeColor::Muted))
                                .bg(bg),
                        ),
                    ])
                })
                .collect();
            let menu_block = Block::default()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_style(Style::default().fg(theme.ratatui_fg(ThemeColor::BorderMuted)));
            f.render_widget(Paragraph::new(menu_lines).block(menu_block), chunks[2]);
        }

        // ── Divider — with scroll indicator when content overflows ────────
        let divider_str = if !self.is_sticky
            && total_lines > viewport_height
            && self.scroll_top < total_lines.saturating_sub(viewport_height)
        {
            let remaining = total_lines
                .saturating_sub(viewport_height)
                .saturating_sub(self.scroll_top);
            let suffix = format!(" ↓{remaining} ");
            let dash_count = (width as usize).saturating_sub(suffix.width());
            format!("{}{}", "─".repeat(dash_count), suffix)
        } else {
            "─".repeat(width as usize)
        };
        let divider_line = Line::from(Span::styled(
            divider_str,
            Style::default().fg(theme.ratatui_fg(ThemeColor::BorderMuted)),
        ));
        f.render_widget(Paragraph::new(vec![divider_line]), chunks[3]);

        // ── Input / spinner ───────────────────────────────────────────────
        const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let input_line = if self.is_thinking {
            let frame = SPINNER[(self.tick as usize) % SPINNER.len()];
            Line::from(vec![
                Span::styled(
                    format!("  {frame} "),
                    Style::default().fg(theme.ratatui_fg(ThemeColor::Accent)),
                ),
                Span::styled(
                    "Thinking…",
                    Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
                ),
            ])
        } else {
            // Cursor blinks every 10 ticks (~500ms).
            let show_cursor = (self.tick / 10) & 1 == 0;
            let cursor = if show_cursor { "▋" } else { " " };
            Line::from(vec![
                Span::styled(
                    "  ❯ ",
                    Style::default().fg(theme.ratatui_fg(ThemeColor::Accent)),
                ),
                Span::raw(self.input_buffer.clone()),
                Span::styled(
                    cursor.to_string(),
                    Style::default().add_modifier(Modifier::REVERSED),
                ),
            ])
        };
        f.render_widget(Paragraph::new(vec![input_line]), chunks[4]);

        // ── Permission approval dialog ────────────────────────────────────
        if let Some(approval) = &self.pending_approval {
            let dialog_height = 8u16;
            let popup_area = Self::centered_rect(62, dialog_height, size);
            f.render_widget(Clear, popup_area);

            let tool_name = approval.tool_name.clone();
            let msg_preview: String = {
                // Flatten control chars inline — no intermediate String allocation.
                let mut chars = approval
                    .message
                    .chars()
                    .map(|c| if c.is_control() { ' ' } else { c });
                let mut s: String = chars.by_ref().take(60).collect();
                if chars.next().is_some() {
                    s.push('…');
                }
                s
            };

            let dialog_lines = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "  Tool: ",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
                    ),
                    Span::styled(
                        tool_name,
                        Style::default()
                            .fg(theme.ratatui_fg(ThemeColor::Accent))
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![Span::styled(
                    format!("  {msg_preview}"),
                    Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
                )]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "  [y] Allow   ",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Success)),
                    ),
                    Span::styled(
                        "[Y] Always",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Success)),
                    ),
                ]),
                Line::from(vec![
                    Span::styled(
                        "  [n] Deny    ",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Error)),
                    ),
                    Span::styled(
                        "[N] Never",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Error)),
                    ),
                ]),
            ];

            let dialog = Paragraph::new(dialog_lines)
                .block(
                    Block::default()
                        .title(" ⚠ Permission Required ")
                        .title_alignment(Alignment::Center)
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(theme.ratatui_fg(ThemeColor::Warning))),
                )
                .wrap(Wrap { trim: false });
            f.render_widget(dialog, popup_area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_interactive_mode_empty_state() {
        let mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.input_buffer.is_empty());
        assert!(mode.messages.is_empty());
        assert!(!mode.running);
    }

    #[tokio::test]
    async fn init_with_initial_message() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions {
            initial_message: Some("Hello".to_string()),
            ..Default::default()
        });
        mode.init().await.unwrap();
        assert_eq!(mode.messages.len(), 1);
        assert_eq!(mode.messages[0].role, MessageRole::User);
        assert_eq!(mode.messages[0].content, "Hello");
    }

    #[tokio::test]
    async fn init_without_initial_message() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.init().await.unwrap();
        assert!(mode.messages.is_empty());
    }

    #[test]
    fn stop_sets_running_false() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.running = true;
        mode.stop();
        assert!(!mode.running);
    }

    #[test]
    fn message_role_equality() {
        assert_eq!(MessageRole::User, MessageRole::User);
        assert_ne!(MessageRole::User, MessageRole::Assistant);
    }

    #[test]
    fn chat_message_clone() {
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "Hello world".to_string(),
        };
        let cloned = msg.clone();
        assert_eq!(cloned.content, "Hello world");
    }

    #[test]
    fn options_default() {
        let opts = InteractiveModeOptions::default();
        assert!(opts.initial_message.is_none());
        assert!(!opts.verbose);
        assert!(opts.model_fallback_message.is_none());
        assert!(opts.migrated_providers.is_empty());
    }

    #[test]
    fn scroll_state_defaults() {
        let mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert_eq!(mode.scroll_top, 0);
        assert!(mode.is_sticky);
    }

    #[test]
    fn compute_message_height_single_line() {
        let msg = ChatMessage {
            role: MessageRole::User,
            content: "short".to_string(),
        };
        // "  ❯ short" fits in 80 cols → 1 row
        assert_eq!(InteractiveMode::compute_message_height(&msg, 80), 1);
    }

    #[test]
    fn compute_message_height_multiline_content() {
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "line one\nline two\nline three".to_string(),
        };
        // 3 content lines → 3 display rows (each fits in wide terminal)
        assert_eq!(InteractiveMode::compute_message_height(&msg, 80), 3);
    }

    #[test]
    fn compute_message_height_wrapping() {
        let msg = ChatMessage {
            role: MessageRole::User,
            content: "a".repeat(100),
        };
        // first_capacity=16, overflow=84 chars at full width 20: 1+ceil(84/20)=6
        let h = InteractiveMode::compute_message_height(&msg, 20);
        assert_eq!(h, 6);
    }

    #[test]
    fn total_content_lines_sums_messages() {
        let msgs = vec![
            ChatMessage {
                role: MessageRole::User,
                content: "hi".to_string(),
            },
            ChatMessage {
                role: MessageRole::Assistant,
                content: "hello".to_string(),
            },
        ];
        let total = InteractiveMode::total_content_lines(&msgs, 80);
        assert_eq!(total, 2);
    }

    #[test]
    fn message_to_lines_empty_content() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::User,
            content: String::new(),
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn message_to_lines_multiline() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "first\nsecond\nthird".to_string(),
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn clamp_scroll_does_not_exceed_max() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.messages.push(ChatMessage {
            role: MessageRole::User,
            content: "hello".to_string(),
        });
        mode.last_terminal_width = 80;
        mode.last_viewport_height = 24;
        mode.scroll_top = 9999;
        mode.clamp_scroll();
        // 1 message = 1 line; max scroll = 1 - 24 = 0 (saturating)
        assert_eq!(mode.scroll_top, 0);
    }

    #[test]
    fn update_completion_matches_finds_commands() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.input_buffer = "/co".to_string();
        mode.update_completion_matches();
        // "compact" and "copy" start with "co"
        assert!(!mode.completion_matches.is_empty());
        assert!(mode.completion_matches.iter().any(|(n, _)| *n == "compact"));
        assert!(mode.completion_matches.iter().any(|(n, _)| *n == "copy"));
    }

    #[test]
    fn update_completion_matches_clears_when_no_slash() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.input_buffer = "hello".to_string();
        mode.update_completion_matches();
        assert!(mode.completion_matches.is_empty());
    }

    #[test]
    fn parse_inline_markdown_bold() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let spans = InteractiveMode::parse_inline_markdown("hello **world** end", &theme);
        // Expect: "hello ", styled "world", " end"
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "hello ");
        assert_eq!(spans[1].content, "world");
        assert!(
            spans[1]
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
        assert_eq!(spans[2].content, " end");
    }

    #[test]
    fn parse_inline_markdown_italic() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let spans = InteractiveMode::parse_inline_markdown("*italic*", &theme);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "italic");
        assert!(
            spans[0]
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::ITALIC)
        );
    }

    #[test]
    fn parse_inline_markdown_code() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let spans = InteractiveMode::parse_inline_markdown("use `foo()` here", &theme);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[1].content, "foo()");
        // Code span should have a background color set
        assert_ne!(spans[1].style.bg, None);
    }

    #[test]
    fn parse_inline_markdown_plain_text() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let spans = InteractiveMode::parse_inline_markdown("plain text", &theme);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "plain text");
    }

    #[test]
    fn message_to_lines_assistant_bold() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "**bold** text".to_string(),
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        assert_eq!(lines.len(), 1);
        // Line has: indicator span + bold span + rest span
        let spans = &lines[0].spans;
        assert!(spans.len() >= 2);
        // Find the bold span
        let bold_span = spans.iter().find(|s| s.content == "bold");
        assert!(bold_span.is_some());
        assert!(
            bold_span
                .unwrap()
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
    }

    #[test]
    fn message_to_lines_assistant_unordered_list() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "- item one".to_string(),
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        assert_eq!(lines.len(), 1);
        let all_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(all_text.contains('•'));
        assert!(!all_text.contains("- "));
    }

    #[test]
    fn message_to_lines_assistant_ordered_list() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "1. first item".to_string(),
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        assert_eq!(lines.len(), 1);
        let all_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        // Number should be preserved
        assert!(all_text.contains("1."));
    }
}
