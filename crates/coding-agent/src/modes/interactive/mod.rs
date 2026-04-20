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
use std::time::Duration;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};

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
        }
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
        // Set up terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        self.running = true;

        // Render initial message if provided
        if let Some(msg) = &self.options.initial_message.clone() {
            if !msg.is_empty() {
                self.messages.push(ChatMessage {
                    role: MessageRole::User,
                    content: msg.clone(),
                });
                // In a real impl we'd send to the agent session here
                self.messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content: "(agent response would appear here)".to_string(),
                });
            }
        }

        loop {
            terminal.draw(|f| self.render(f))?;

            if event::poll(Duration::from_millis(16))? {
                if let Event::Key(key) = event::read()? {
                    match (key.code, key.modifiers) {
                        // Ctrl+C / Ctrl+D — quit
                        (KeyCode::Char('c'), KeyModifiers::CONTROL)
                        | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                            self.running = false;
                            break;
                        }
                        // Enter — submit input
                        (KeyCode::Enter, _) => {
                            let input = std::mem::take(&mut self.input_buffer);
                            if !input.trim().is_empty() {
                                self.messages.push(ChatMessage {
                                    role: MessageRole::User,
                                    content: input.clone(),
                                });
                                // In a real impl: send to agent session and await
                                self.messages.push(ChatMessage {
                                    role: MessageRole::Assistant,
                                    content: format!("(response to: {input})"),
                                });
                            }
                        }
                        // Backspace
                        (KeyCode::Backspace, _) => {
                            self.input_buffer.pop();
                        }
                        // Regular character input
                        (KeyCode::Char(c), _) => {
                            self.input_buffer.push(c);
                        }
                        _ => {}
                    }
                }
            }

            if !self.running {
                break;
            }
        }

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        Ok(())
    }

    /// Stop the interactive mode (e.g., for startup benchmarks).
    pub fn stop(&mut self) {
        self.running = false;
    }

    /// Render the TUI frame.
    fn render(&self, f: &mut ratatui::Frame) {
        let size = f.area();

        // Split into message area and input area
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)])
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

        // Input area
        let input_widget = Paragraph::new(self.input_buffer.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Input (Enter to send, Ctrl+C to quit) "),
        );
        f.render_widget(input_widget, chunks[1]);
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
