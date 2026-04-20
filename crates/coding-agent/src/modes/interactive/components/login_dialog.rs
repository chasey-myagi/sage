//! Login dialog component.
//!
//! Translated from `components/login-dialog.ts`.
//!
//! Provides an interactive OAuth login dialog.

use tui::components::spacer::Spacer;
use tui::components::text::Text;
use tui::tui::{Component, Container};

use crate::modes::interactive::components::dynamic_border::DynamicBorder;
use crate::modes::interactive::components::keybinding_hints::key_hint;
use crate::modes::interactive::theme::{ThemeColor, get_theme};

/// State of the login dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginDialogState {
    /// Showing the initial prompt with URL.
    Prompt { url: String },
    /// Waiting for user to paste an auth code.
    AwaitingCode,
    /// Login completed (success or failure).
    Done {
        success: bool,
        message: Option<String>,
    },
}

/// Login dialog component.
pub struct LoginDialogComponent {
    provider_name: String,
    state: LoginDialogState,
    input_buffer: String,
    on_complete: Option<Box<dyn Fn(bool, Option<String>) + Send>>,
}

impl LoginDialogComponent {
    pub fn new(provider_name: impl Into<String>) -> Self {
        Self {
            provider_name: provider_name.into(),
            state: LoginDialogState::Prompt { url: String::new() },
            input_buffer: String::new(),
            on_complete: None,
        }
    }

    pub fn set_on_complete<F: Fn(bool, Option<String>) + Send + 'static>(&mut self, f: F) {
        self.on_complete = Some(Box::new(f));
    }

    pub fn set_auth_url(&mut self, url: String) {
        self.state = LoginDialogState::Prompt { url };
    }

    pub fn set_awaiting_code(&mut self) {
        self.state = LoginDialogState::AwaitingCode;
        self.input_buffer.clear();
    }

    pub fn complete(&mut self, success: bool, message: Option<String>) {
        self.state = LoginDialogState::Done {
            success,
            message: message.clone(),
        };
        if let Some(ref f) = self.on_complete {
            f(success, message);
        }
    }

    pub fn handle_key(&mut self, key: &str) -> bool {
        match &self.state {
            LoginDialogState::Prompt { .. } => {
                if key == "\r" || key == "\n" || key == " " {
                    self.set_awaiting_code();
                    return true;
                }
                if key == "\x1b" {
                    self.complete(false, Some("Cancelled".to_string()));
                    return true;
                }
            }
            LoginDialogState::AwaitingCode => {
                if key == "\r" || key == "\n" {
                    let code = self.input_buffer.trim().to_string();
                    if !code.is_empty() {
                        // In real impl, this would trigger OAuth token exchange
                        self.complete(true, None);
                    }
                    return true;
                }
                if key == "\x1b" {
                    self.complete(false, Some("Cancelled".to_string()));
                    return true;
                }
                if key == "\x7f" {
                    self.input_buffer.pop();
                    return true;
                }
                if key.len() == 1 {
                    self.input_buffer.push_str(key);
                    return true;
                }
            }
            LoginDialogState::Done { .. } => {}
        }
        false
    }

    pub fn get_code(&self) -> &str {
        &self.input_buffer
    }
}

impl Component for LoginDialogComponent {
    fn render(&self, width: u16) -> Vec<String> {
        let t = get_theme();
        let mut container = Container::new();

        container.add_child(Box::new(DynamicBorder::new()));

        let title = t.fg(
            ThemeColor::Warning,
            &format!("Login to {}", self.provider_name),
        );
        container.add_child(Box::new(Text::new(title, 1, 0)));
        container.add_child(Box::new(Spacer::new(1)));

        match &self.state {
            LoginDialogState::Prompt { url } => {
                if !url.is_empty() {
                    container.add_child(Box::new(Text::new(
                        t.fg(ThemeColor::Muted, "Visit this URL to authenticate:"),
                        1,
                        0,
                    )));
                    container.add_child(Box::new(Text::new(t.fg(ThemeColor::Accent, url), 1, 0)));
                    container.add_child(Box::new(Spacer::new(1)));
                }
                container.add_child(Box::new(Text::new(
                    t.fg(
                        ThemeColor::Muted,
                        "Press Enter or Space to open the browser...",
                    ),
                    1,
                    0,
                )));
                container.add_child(Box::new(Text::new(key_hint("Esc", "cancel"), 1, 0)));
            }
            LoginDialogState::AwaitingCode => {
                container.add_child(Box::new(Text::new(
                    t.fg(ThemeColor::Muted, "Paste the authorization code:"),
                    1,
                    0,
                )));
                let input_line = format!(
                    "{} {}█",
                    t.fg(ThemeColor::Dim, ">"),
                    t.fg(ThemeColor::Accent, &self.input_buffer)
                );
                container.add_child(Box::new(Text::new(input_line, 1, 0)));
                container.add_child(Box::new(Text::new(key_hint("Enter", "submit"), 1, 0)));
            }
            LoginDialogState::Done { success, message } => {
                if *success {
                    container.add_child(Box::new(Text::new(
                        t.fg(ThemeColor::Success, "✓ Login successful"),
                        1,
                        0,
                    )));
                } else {
                    let msg = message.as_deref().unwrap_or("Login failed");
                    container.add_child(Box::new(Text::new(t.fg(ThemeColor::Error, msg), 1, 0)));
                }
            }
        }

        container.add_child(Box::new(DynamicBorder::new()));
        container.render(width)
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_provider_name() {
        let comp = LoginDialogComponent::new("Anthropic");
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("Anthropic"));
    }

    #[test]
    fn esc_cancels_from_prompt() {
        let mut comp = LoginDialogComponent::new("Test");
        comp.set_auth_url("https://example.com/auth".to_string());
        comp.handle_key("\x1b");
        assert!(matches!(
            comp.state,
            LoginDialogState::Done { success: false, .. }
        ));
    }

    #[test]
    fn enter_transitions_to_awaiting_code() {
        let mut comp = LoginDialogComponent::new("Test");
        comp.set_auth_url("https://example.com/auth".to_string());
        comp.handle_key("\r");
        assert!(matches!(comp.state, LoginDialogState::AwaitingCode));
    }
}
