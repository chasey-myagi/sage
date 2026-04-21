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
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyModifiers},
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
            self.spawn_agent(msg);
        }

        let mut event_stream = EventStream::new();

        loop {
            // Drain agent response deltas
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
                        Err(mpsc::error::TryRecvError::Empty) => break,
                        Err(mpsc::error::TryRecvError::Disconnected) => {
                            self.agent_rx = None;
                            self.is_thinking = false;
                            break;
                        }
                    }
                }
            }

            terminal.draw(|f| self.render(f))?;

            tokio::select! {
                maybe_event = event_stream.next() => {
                    let Some(Ok(Event::Key(key))) = maybe_event else { continue };
                    match (key.code, key.modifiers) {
                        (KeyCode::Char('c'), KeyModifiers::CONTROL)
                        | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                            self.running = false;
                            break;
                        }
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
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
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
                "default".to_string(),
            )
            .await;
        });
    }

    /// Stop the interactive mode (e.g., for startup benchmarks).
    pub fn stop(&mut self) {
        self.running = false;
    }

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

    /// Render the TUI frame.
    fn render(&self, f: &mut ratatui::Frame) {
        let size = f.area();

        // Three sections: messages | statusbar | input
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(1),
                Constraint::Length(3),
            ])
            .split(size);

        // Message history
        let messages: Vec<Line> = self
            .messages
            .iter()
            .map(|m| {
                let prefix = match m.role {
                    MessageRole::User => Span::styled("You: ", Style::default().fg(Color::Cyan)),
                    MessageRole::Assistant => {
                        Span::styled("Sage: ", Style::default().fg(Color::Green))
                    }
                    MessageRole::System => {
                        Span::styled("System: ", Style::default().fg(Color::Yellow))
                    }
                };
                Line::from(vec![prefix, Span::raw(m.content.clone())])
            })
            .collect();

        let messages_widget = Paragraph::new(messages)
            .block(Block::default().borders(Borders::ALL).title(" Sage "))
            .wrap(Wrap { trim: false });
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
        let status_widget = Paragraph::new(status_text).style(Style::default().fg(Color::DarkGray));
        f.render_widget(status_widget, chunks[1]);

        let input_title = if self.is_thinking {
            " Thinking… (Ctrl+C to quit) "
        } else {
            " Input (Enter to send, Ctrl+C to quit) "
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
}
