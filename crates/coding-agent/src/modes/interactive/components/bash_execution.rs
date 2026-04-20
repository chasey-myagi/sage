//! Bash execution component.
//!
//! Translated from `components/bash-execution.ts`.
//!
//! Renders a bash command execution with streaming output.

use tui::components::spacer::Spacer;
use tui::components::text::Text;
use tui::tui::{Component, Container};

use crate::modes::interactive::components::dynamic_border::DynamicBorder;
use crate::modes::interactive::components::keybinding_hints::{key_hint, key_text};
use crate::modes::interactive::components::visual_truncate::truncate_to_visual_lines;
use crate::modes::interactive::theme::{ThemeColor, get_theme};

const PREVIEW_LINES: usize = 20;

/// Status of a bash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BashStatus {
    Running,
    Complete,
    Cancelled,
    Error,
}

/// Bash execution component that shows command + streaming output.
pub struct BashExecutionComponent {
    command: String,
    output_lines: Vec<String>,
    status: BashStatus,
    exit_code: Option<i32>,
    expanded: bool,
    exclude_from_context: bool,
}

impl BashExecutionComponent {
    pub fn new(command: impl Into<String>, exclude_from_context: bool) -> Self {
        Self {
            command: command.into(),
            output_lines: Vec::new(),
            status: BashStatus::Running,
            exit_code: None,
            expanded: false,
            exclude_from_context,
        }
    }

    pub fn append_output(&mut self, chunk: &str) {
        for line in chunk.split('\n') {
            self.output_lines.push(line.to_string());
        }
    }

    pub fn set_complete(&mut self, exit_code: i32) {
        self.exit_code = Some(exit_code);
        self.status = if exit_code == 0 {
            BashStatus::Complete
        } else {
            BashStatus::Error
        };
    }

    pub fn set_cancelled(&mut self) {
        self.status = BashStatus::Cancelled;
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
    }

    pub fn is_running(&self) -> bool {
        self.status == BashStatus::Running
    }

    fn color_key(&self) -> ThemeColor {
        if self.exclude_from_context {
            ThemeColor::Dim
        } else {
            ThemeColor::BashMode
        }
    }
}

impl Component for BashExecutionComponent {
    fn render(&self, width: u16) -> Vec<String> {
        let t = get_theme();
        let color_key = self.color_key();
        let mut container = Container::new();

        container.add_child(Box::new(Spacer::new(1)));

        // Top border
        let ck = color_key.clone();
        container.add_child(Box::new(DynamicBorder::with_color({
            let t2 = t.clone();
            let ck2 = ck.clone();
            move |s: &str| t2.fg(ck2.clone(), s)
        })));

        // Command header
        let cmd_line = t.bold(&t.fg(color_key.clone(), &format!("$ {}", self.command)));
        container.add_child(Box::new(Text::new(cmd_line, 1, 0)));

        // Output
        if !self.output_lines.is_empty() {
            let all_output = self.output_lines.join("\n");

            if self.expanded {
                for line in &self.output_lines {
                    let colored = t.fg(ThemeColor::ToolOutput, line);
                    container.add_child(Box::new(Text::new(colored, 1, 0)));
                }
            } else {
                let result = truncate_to_visual_lines(&all_output, PREVIEW_LINES, width, 1);
                if result.skipped_count > 0 {
                    container.add_child(Box::new(Text::new(
                        t.fg(
                            ThemeColor::Dim,
                            &format!("… {} lines hidden", result.skipped_count),
                        ),
                        1,
                        0,
                    )));
                }
                for line in &result.visual_lines {
                    let colored = t.fg(ThemeColor::ToolOutput, line);
                    container.add_child(Box::new(Text::new(colored, 1, 0)));
                }
            }
        }

        // Status
        match &self.status {
            BashStatus::Running => {
                let hint_text = format!("Running... ({} to cancel)", key_text("ctrl+c"));
                container.add_child(Box::new(Text::new(
                    t.fg(ThemeColor::Muted, &hint_text),
                    1,
                    0,
                )));
            }
            BashStatus::Complete => {
                let exit = self.exit_code.unwrap_or(0);
                if exit != 0 {
                    container.add_child(Box::new(Text::new(
                        t.fg(ThemeColor::Error, &format!("Exit {exit}")),
                        1,
                        0,
                    )));
                }
            }
            BashStatus::Error => {
                let exit = self.exit_code.unwrap_or(-1);
                container.add_child(Box::new(Text::new(
                    t.fg(ThemeColor::Error, &format!("Exit {exit}")),
                    1,
                    0,
                )));
            }
            BashStatus::Cancelled => {
                container.add_child(Box::new(Text::new(
                    t.fg(ThemeColor::Warning, "Cancelled"),
                    1,
                    0,
                )));
            }
        }

        // Expand/collapse hint when output is available
        if !self.output_lines.is_empty() && self.status != BashStatus::Running {
            let hint = if self.expanded {
                key_hint("ctrl+e", "collapse")
            } else {
                key_hint("ctrl+e", "expand")
            };
            container.add_child(Box::new(Text::new(hint, 1, 0)));
        }

        // Bottom border
        container.add_child(Box::new(DynamicBorder::with_color({
            let t3 = t.clone();
            let ck3 = color_key.clone();
            move |s: &str| t3.fg(ck3.clone(), s)
        })));

        container.render(width)
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_command() {
        let comp = BashExecutionComponent::new("ls -la", false);
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("ls -la"), "Expected command in: {text:?}");
    }

    #[test]
    fn shows_output_when_complete() {
        let mut comp = BashExecutionComponent::new("echo hello", false);
        comp.append_output("hello\n");
        comp.set_complete(0);
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("hello"), "Expected output in: {text:?}");
    }

    #[test]
    fn cancelled_shows_cancelled_label() {
        let mut comp = BashExecutionComponent::new("sleep 10", false);
        comp.set_cancelled();
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("Cancelled"));
    }
}
