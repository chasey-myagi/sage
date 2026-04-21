//! Interactive TUI mode for the coding agent.
//!
//! Translated from pi-mono `packages/coding-agent/src/modes/interactive/interactive-mode.ts`.
//!
//! This is the full-screen TUI mode launched when `sage` is invoked without
//! `--print` / `--mode json|rpc`. The implementation here is a structural
//! skeleton; the ratatui rendering details are fleshed out in the `tui` crate.

pub mod components;
pub mod theme;

use std::io;

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
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use tokio::sync::mpsc;

use crate::agent_session::AgentDelta;

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
    Tool { name: String, pending: bool, success: bool },
    /// A fatal agent error shown inline.
    Error,
}

impl InteractiveMode {
    /// Create a new InteractiveMode with the given options.
    pub fn new(options: InteractiveModeOptions) -> Self {
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
                        Ok(AgentDelta::TurnUsage { usage, model, is_fast }) => {
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
                        Ok(AgentDelta::ToolEnd { name, success, output_preview }) => {
                            let pos = self.messages.iter().rposition(|m| {
                                matches!(&m.role, MessageRole::Tool { name: n, pending: true, .. } if n == &name)
                            });
                            if let Some(idx) = pos {
                                if !output_preview.is_empty() {
                                    let existing = self.messages[idx].content.clone();
                                    self.messages[idx].content =
                                        format!("{existing} · {}", output_preview.chars().take(80).collect::<String>());
                                }
                                self.messages[idx].role =
                                    MessageRole::Tool { name, pending: false, success };
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
                self.is_thinking = false;
            }

            // sticky scroll is applied inside render with fresh dimensions
            terminal.draw(|f| self.render(f))?;

            tokio::select! {
                maybe_event = event_stream.next() => {
                    let Some(Ok(event)) = maybe_event else { continue };
                    match event {
                        Event::Key(key) => {
                            match (key.code, key.modifiers) {
                                (KeyCode::Char('c'), KeyModifiers::CONTROL)
                                | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
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
                                (KeyCode::Char('G'), KeyModifiers::NONE)
                                    if self.input_buffer.is_empty() =>
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
                                // Input handling
                                (KeyCode::Enter, _) if !self.is_thinking => {
                                    let input = std::mem::take(&mut self.input_buffer);
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
                                }
                                (KeyCode::Char(c), _) => {
                                    self.input_buffer.push(c);
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
        execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
        terminal.show_cursor()?;
        Ok(())
    }

    fn spawn_agent(&mut self, message: String) {
        let (tx, rx) = mpsc::unbounded_channel::<AgentDelta>();
        self.agent_rx = Some(rx);
        self.is_thinking = true;

        let provider_id = self.provider_id.clone();
        let model_id = self.model_id.clone();

        tokio::spawn(async move {
            let _ = crate::agent_session::run_agent_session_to_channel(
                message,
                model_id,
                provider_id,
                None,
                tx,
            )
            .await;
        });
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
        let prefix_len: u16 = 4; // "  ❯ " / "  ◆ " / "  ◆ " — all 4 display cols
        let effective = inner_width.saturating_sub(prefix_len).max(1);
        let count: u16 = msg
            .content
            .lines()
            .map(|line| {
                let n = line.chars().count() as u16;
                if n == 0 { 1 } else { n.div_ceil(effective) }
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

    /// Convert a `ChatMessage` to ratatui `Line`s, handling embedded newlines.
    ///
    /// First content line: `  ❯ {text}` (user) or `  ◆ {text}` (assistant).
    /// Subsequent lines: indented by 4 spaces to align under the text.
    fn message_to_lines(msg: &ChatMessage) -> Vec<Line<'static>> {
        let (indicator, style) = match &msg.role {
            MessageRole::User => ("❯", Style::default().fg(Color::Cyan)),
            MessageRole::Assistant => ("◆", Style::default().fg(Color::Green)),
            MessageRole::System => ("◆", Style::default().fg(Color::Yellow)),
            MessageRole::Tool { pending: true, .. } => {
                ("⏺", Style::default().fg(Color::Yellow))
            }
            MessageRole::Tool { success: true, .. } => {
                ("✓", Style::default().fg(Color::DarkGray))
            }
            MessageRole::Tool { .. } => ("✘", Style::default().fg(Color::Red)),
            MessageRole::Error => ("✘", Style::default().fg(Color::Red)),
        };
        let prefix = format!("  {indicator} ");
        let indent = "    ".to_string(); // 4 spaces — matches prefix display width
        let mut lines: Vec<Line<'static>> = Vec::new();

        for (i, text_line) in msg.content.lines().enumerate() {
            let lead = if i == 0 {
                Span::styled(prefix.clone(), style)
            } else {
                Span::raw(indent.clone())
            };
            lines.push(Line::from(vec![lead, Span::raw(text_line.to_string())]));
        }

        if lines.is_empty() {
            lines.push(Line::from(vec![Span::styled(prefix, style)]));
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

    // ── Render ──────────────────────────────────────────────────────────────

    /// Render the TUI frame — CC-style borderless layout.
    ///
    /// ```
    /// ┌ header: model name ────────────────── ↑Xk ↓Xk  $X.XXXX ┐
    /// │ messages area (no border)                                 │
    /// │   ❯  user input                                           │
    /// │   ◆  assistant response                                   │
    /// ├ ─────────────────────────────────────────────────────── ─ ┤
    /// └   ❯  input buffer  (or  ⠋  Thinking…)                   ┘
    /// ```
    fn render(&mut self, f: &mut ratatui::Frame) {
        let size = f.area();
        let width = size.width;

        // Layout: header(1) | messages(∞) | divider(1) | input(1)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // header
                Constraint::Min(0),    // messages
                Constraint::Length(1), // divider
                Constraint::Length(1), // input prompt
            ])
            .split(size);

        let viewport_height = chunks[1].height;

        // Cache dimensions used by key/mouse handlers between frames.
        self.last_terminal_width = width;
        self.last_viewport_height = viewport_height;

        // Sticky scroll: pin to bottom as new content arrives.
        if self.is_sticky {
            let total = Self::total_content_lines(&self.messages, width);
            self.scroll_top = total.saturating_sub(viewport_height);
        }

        // ── Header ────────────────────────────────────────────────────────
        let model_label = self.model_id.as_deref().unwrap_or("claude");
        let header_left = format!("  {model_label}");
        let stats = format!(
            "↑{}  ↓{}  {}  ",
            Self::format_tokens(self.session_input_tokens),
            Self::format_tokens(self.session_output_tokens),
            Self::format_cost(self.session_cost_usd),
        );
        // Right-align stats; pad between left label and right stats.
        let left_len = header_left.chars().count() as u16;
        let stats_len = stats.chars().count() as u16;
        let gap = width.saturating_sub(left_len + stats_len);
        let header_line = Line::from(vec![
            Span::styled(
                header_left,
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ".repeat(gap as usize)),
            Span::styled(stats, Style::default().fg(Color::DarkGray)),
        ]);
        f.render_widget(Paragraph::new(vec![header_line]), chunks[0]);

        // ── Messages ──────────────────────────────────────────────────────
        let mut message_lines: Vec<Line> = self
            .messages
            .iter()
            .flat_map(|m| Self::message_to_lines(m))
            .collect();

        // Empty state: welcome prompt when no conversation has started.
        if message_lines.is_empty() && !self.is_thinking {
            message_lines = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("  ◆ ", Style::default().fg(Color::Green)),
                    Span::styled(
                        "What can I help you with?",
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
            ];
        }

        let total_lines = Self::total_content_lines(&self.messages, width);
        let messages_widget = Paragraph::new(message_lines)
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_top, 0));
        f.render_widget(messages_widget, chunks[1]);

        // ── Divider — with scroll indicator when content overflows ────────
        let divider_str = if !self.is_sticky
            && total_lines > viewport_height
            && self.scroll_top < total_lines.saturating_sub(viewport_height)
        {
            let remaining = total_lines
                .saturating_sub(viewport_height)
                .saturating_sub(self.scroll_top);
            let suffix = format!(" ↓{remaining} ");
            let dash_count = (width as usize).saturating_sub(suffix.len());
            format!("{}{}", "─".repeat(dash_count), suffix)
        } else {
            "─".repeat(width as usize)
        };
        let divider_line = Line::from(Span::styled(
            divider_str,
            Style::default().fg(Color::DarkGray),
        ));
        f.render_widget(Paragraph::new(vec![divider_line]), chunks[2]);

        // ── Input / spinner ───────────────────────────────────────────────
        const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let input_line = if self.is_thinking {
            let frame = SPINNER[(self.tick as usize) % SPINNER.len()];
            Line::from(vec![
                Span::styled(format!("  {frame} "), Style::default().fg(Color::Green)),
                Span::styled("Thinking…", Style::default().fg(Color::DarkGray)),
            ])
        } else {
            Line::from(vec![
                Span::styled("  ❯ ", Style::default().fg(Color::Green)),
                Span::raw(self.input_buffer.clone()),
            ])
        };
        f.render_widget(Paragraph::new(vec![input_line]), chunks[3]);
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
        // prefix = 4, effective = 20-4 = 16 cols; ceil(100/16) = 7
        let h = InteractiveMode::compute_message_height(&msg, 20);
        assert_eq!(h, 7);
    }

    #[test]
    fn total_content_lines_sums_messages() {
        let msgs = vec![
            ChatMessage { role: MessageRole::User, content: "hi".to_string() },
            ChatMessage { role: MessageRole::Assistant, content: "hello".to_string() },
        ];
        let total = InteractiveMode::total_content_lines(&msgs, 80);
        assert_eq!(total, 2);
    }

    #[test]
    fn message_to_lines_empty_content() {
        let msg = ChatMessage { role: MessageRole::User, content: String::new() };
        let lines = InteractiveMode::message_to_lines(&msg);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn message_to_lines_multiline() {
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "first\nsecond\nthird".to_string(),
        };
        let lines = InteractiveMode::message_to_lines(&msg);
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
}
