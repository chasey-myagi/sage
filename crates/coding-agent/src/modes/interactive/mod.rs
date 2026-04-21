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
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
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
    /// Viewport height cached from last render (lines, excluding borders).
    last_viewport_height: u16,
    /// Terminal width cached from last render (columns, including borders).
    last_terminal_width: u16,
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
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
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
    /// Mirrors what ratatui's Paragraph+Wrap would produce: each content line
    /// is divided into chunks of `effective_width` columns. We count prefix
    /// characters as part of the first content line's width budget.
    fn compute_message_height(msg: &ChatMessage, inner_width: u16) -> u16 {
        let prefix_len: u16 = match msg.role {
            MessageRole::User => 5,      // "You: "
            MessageRole::Assistant => 6, // "Sage: "
            MessageRole::System => 8,    // "System: "
        };
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
    /// The first content line is prefixed with the role label; subsequent lines
    /// are indented by the same width so wrapped text aligns visually.
    fn message_to_lines(msg: &ChatMessage) -> Vec<Line<'static>> {
        let (prefix, style) = match msg.role {
            MessageRole::User => ("You: ", Style::default().fg(Color::Cyan)),
            MessageRole::Assistant => ("Sage: ", Style::default().fg(Color::Green)),
            MessageRole::System => ("System: ", Style::default().fg(Color::Yellow)),
        };
        let indent = " ".repeat(prefix.len());
        let mut lines: Vec<Line<'static>> = Vec::new();

        for (i, text_line) in msg.content.lines().enumerate() {
            let lead = if i == 0 {
                Span::styled(prefix.to_string(), style)
            } else {
                Span::raw(indent.clone())
            };
            lines.push(Line::from(vec![lead, Span::raw(text_line.to_string())]));
        }

        if lines.is_empty() {
            lines.push(Line::from(vec![Span::styled(prefix.to_string(), style)]));
        }

        lines
    }

    /// Clamp `scroll_top` so we never scroll past the last line of content.
    fn clamp_scroll(&mut self) {
        let inner_width = self.last_terminal_width.saturating_sub(2);
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

    /// Render the TUI frame. Updates cached dimensions and applies sticky scroll.
    fn render(&mut self, f: &mut ratatui::Frame) {
        let size = f.area();
        let inner_width = size.width.saturating_sub(2); // inside borders

        // Three sections: messages | statusbar | input
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1), Constraint::Length(3)])
            .split(size);

        let viewport_height = chunks[0].height.saturating_sub(2);

        // Cache dimensions used by key/mouse handlers between frames.
        self.last_terminal_width = size.width;
        self.last_viewport_height = viewport_height;

        // Sticky scroll: always show the bottom of content when new text arrives.
        if self.is_sticky {
            let total = Self::total_content_lines(&self.messages, inner_width);
            self.scroll_top = total.saturating_sub(viewport_height);
        }

        // Build multi-line-aware flat line list from all messages.
        let message_lines: Vec<Line> = self
            .messages
            .iter()
            .flat_map(|m| Self::message_to_lines(m))
            .collect();

        // Scroll-position indicator shown in the message panel title.
        let total_lines = Self::total_content_lines(&self.messages, inner_width);
        let title = if self.is_sticky {
            " Sage [↓] ".to_string()
        } else if total_lines > viewport_height {
            let denom = total_lines.saturating_sub(viewport_height);
            let pct = if denom > 0 {
                ((self.scroll_top as f32 / denom as f32) * 100.0).min(100.0) as u8
            } else {
                100
            };
            format!(" Sage [{pct}%] ")
        } else {
            " Sage ".to_string()
        };

        let messages_widget = Paragraph::new(message_lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_top, 0));
        f.render_widget(messages_widget, chunks[0]);

        // Status bar: token counts and cost
        let status_text = if self.session_input_tokens == 0 && self.session_output_tokens == 0 {
            " ↑0 ↓0 tokens | $0.0000".to_string()
        } else {
            format!(
                " ↑{} ↓{} tokens | {}",
                Self::format_tokens(self.session_input_tokens),
                Self::format_tokens(self.session_output_tokens),
                Self::format_cost(self.session_cost_usd),
            )
        };
        let status_widget =
            Paragraph::new(status_text).style(Style::default().fg(Color::DarkGray));
        f.render_widget(status_widget, chunks[1]);

        // Input box with context-sensitive hints.
        let scroll_hint = if self.input_buffer.is_empty() {
            "j/k·PageUp/Dn·G=bottom · "
        } else {
            ""
        };
        let input_title = if self.is_thinking {
            format!(" Thinking… ({scroll_hint}Ctrl+C quit) ")
        } else {
            format!(" Input ({scroll_hint}Enter send · Ctrl+C quit) ")
        };
        let input_widget = Paragraph::new(self.input_buffer.as_str())
            .block(Block::default().borders(Borders::ALL).title(input_title));
        f.render_widget(input_widget, chunks[2]);
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
        // "You: short" fits in 80 cols → 1 row
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
        // prefix = 5, effective = 20-5 = 15 cols; 100 chars / 15 = 7 rows
        let h = InteractiveMode::compute_message_height(&msg, 20);
        assert_eq!(h, 7); // ceil(100/15) = 7
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
