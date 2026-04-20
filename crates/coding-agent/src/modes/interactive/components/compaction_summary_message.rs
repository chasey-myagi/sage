//! Compaction summary message component.
//!
//! Translated from `components/compaction-summary-message.ts`.

use tui::components::markdown::{DefaultTextStyle, Markdown, MarkdownTheme};
use tui::components::spacer::Spacer;
use tui::components::text::Text;
use tui::tui::{Component, Container};

use crate::modes::interactive::components::keybinding_hints::key_text;
use crate::modes::interactive::theme::{ThemeColor, get_theme};

/// A compaction summary message.
#[derive(Debug, Clone)]
pub struct CompactionSummaryMessage {
    pub tokens_before: u64,
    pub summary: String,
}

/// Renders a compaction summary with collapsed/expanded state.
pub struct CompactionSummaryMessageComponent {
    message: CompactionSummaryMessage,
    expanded: bool,
}

impl CompactionSummaryMessageComponent {
    pub fn new(message: CompactionSummaryMessage) -> Self {
        Self {
            message,
            expanded: false,
        }
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
    }
}

fn build_md_theme() -> MarkdownTheme {
    let t = get_theme();
    MarkdownTheme {
        heading: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdHeading, s)
        }),
        link: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdLink, s)
        }),
        link_url: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdLinkUrl, s)
        }),
        code: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdCode, s)
        }),
        code_block: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdCodeBlock, s)
        }),
        code_block_border: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdCodeBlockBorder, s)
        }),
        quote: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdQuote, s)
        }),
        quote_border: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdQuoteBorder, s)
        }),
        hr: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdHr, s)
        }),
        list_bullet: Box::new({
            let t2 = t.clone();
            move |s: &str| t2.fg(ThemeColor::MdListBullet, s)
        }),
        bold: Box::new(|s: &str| format!("\x1b[1m{s}\x1b[22m")),
        italic: Box::new(|s: &str| format!("\x1b[3m{s}\x1b[23m")),
        strikethrough: Box::new(|s: &str| format!("\x1b[9m{s}\x1b[29m")),
        underline: Box::new(|s: &str| format!("\x1b[4m{s}\x1b[24m")),
        code_block_indent: None,
        highlight_code: None,
    }
}

impl Component for CompactionSummaryMessageComponent {
    fn render(&self, width: u16) -> Vec<String> {
        let t = get_theme();
        let mut container = Container::new();

        let label = t.bold(&t.fg(ThemeColor::CustomMessageLabel, "[compaction]"));
        container.add_child(Box::new(Text::new(label, 1, 0)));
        container.add_child(Box::new(Spacer::new(1)));

        if self.expanded {
            let token_str = format_number(self.message.tokens_before);
            let header = format!("**Compacted from {token_str} tokens**\n\n");
            let style = DefaultTextStyle {
                color: Some(Box::new({
                    let t2 = t.clone();
                    move |s: &str| t2.fg(ThemeColor::CustomMessageText, s)
                })),
                bg_color: None,
                bold: false,
                italic: false,
                strikethrough: false,
                underline: false,
            };
            container.add_child(Box::new(Markdown::new(
                format!("{}{}", header, self.message.summary),
                1,
                0,
                build_md_theme(),
                Some(style),
            )));
        } else {
            let token_str = format_number(self.message.tokens_before);
            let hint = key_text("ctrl+e");
            let line = format!(
                "{}{}{}",
                t.fg(
                    ThemeColor::CustomMessageText,
                    &format!("Compacted from {token_str} tokens (")
                ),
                t.fg(ThemeColor::Dim, &hint),
                t.fg(ThemeColor::CustomMessageText, " to expand)"),
            );
            container.add_child(Box::new(Text::new(line, 1, 0)));
        }

        container.render(width)
    }

    fn invalidate(&mut self) {}
}

/// Format a number with locale-style grouping (e.g. 1,234,567).
fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result.chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_number_small() {
        assert_eq!(format_number(1234), "1,234");
        assert_eq!(format_number(1_234_567), "1,234,567");
        assert_eq!(format_number(100), "100");
    }

    #[test]
    fn collapsed_shows_token_count() {
        let msg = CompactionSummaryMessage {
            tokens_before: 50_000,
            summary: "Context was compacted".to_string(),
        };
        let comp = CompactionSummaryMessageComponent::new(msg);
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("50,000"));
    }

    #[test]
    fn expanded_shows_summary() {
        let msg = CompactionSummaryMessage {
            tokens_before: 50_000,
            summary: "Context was compacted".to_string(),
        };
        let mut comp = CompactionSummaryMessageComponent::new(msg);
        comp.set_expanded(true);
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("Context was compacted"));
    }
}
