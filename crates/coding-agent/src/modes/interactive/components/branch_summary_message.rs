//! Branch summary message component.
//!
//! Translated from `components/branch-summary-message.ts`.

use tui::tui::{Component, Container};
use tui::components::spacer::Spacer;
use tui::components::text::Text;
use tui::components::markdown::{Markdown, MarkdownTheme, DefaultTextStyle};

use crate::modes::interactive::theme::{get_theme, ThemeColor};
use crate::modes::interactive::components::keybinding_hints::key_text;

/// A branch summary message produced after branching.
#[derive(Debug, Clone)]
pub struct BranchSummaryMessage {
    pub branch: String,
    pub summary: String,
}

/// Renders a branch summary with collapsed/expanded state.
pub struct BranchSummaryMessageComponent {
    message: BranchSummaryMessage,
    expanded: bool,
}

impl BranchSummaryMessageComponent {
    pub fn new(message: BranchSummaryMessage) -> Self {
        Self { message, expanded: false }
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
    }
}

fn build_md_theme() -> MarkdownTheme {
    let t = get_theme();
    MarkdownTheme {
        heading: Box::new({ let t2 = t.clone(); move |s: &str| t2.fg(ThemeColor::MdHeading, s) }),
        link: Box::new({ let t2 = t.clone(); move |s: &str| t2.fg(ThemeColor::MdLink, s) }),
        link_url: Box::new({ let t2 = t.clone(); move |s: &str| t2.fg(ThemeColor::MdLinkUrl, s) }),
        code: Box::new({ let t2 = t.clone(); move |s: &str| t2.fg(ThemeColor::MdCode, s) }),
        code_block: Box::new({ let t2 = t.clone(); move |s: &str| t2.fg(ThemeColor::MdCodeBlock, s) }),
        code_block_border: Box::new({ let t2 = t.clone(); move |s: &str| t2.fg(ThemeColor::MdCodeBlockBorder, s) }),
        quote: Box::new({ let t2 = t.clone(); move |s: &str| t2.fg(ThemeColor::MdQuote, s) }),
        quote_border: Box::new({ let t2 = t.clone(); move |s: &str| t2.fg(ThemeColor::MdQuoteBorder, s) }),
        hr: Box::new({ let t2 = t.clone(); move |s: &str| t2.fg(ThemeColor::MdHr, s) }),
        list_bullet: Box::new({ let t2 = t.clone(); move |s: &str| t2.fg(ThemeColor::MdListBullet, s) }),
        bold: Box::new(|s: &str| format!("\x1b[1m{s}\x1b[22m")),
        italic: Box::new(|s: &str| format!("\x1b[3m{s}\x1b[23m")),
        strikethrough: Box::new(|s: &str| format!("\x1b[9m{s}\x1b[29m")),
        underline: Box::new(|s: &str| format!("\x1b[4m{s}\x1b[24m")),
        code_block_indent: None,
        highlight_code: None,
    }
}

impl Component for BranchSummaryMessageComponent {
    fn render(&self, width: u16) -> Vec<String> {
        let t = get_theme();
        let mut container = Container::new();

        let label = t.bold(&t.fg(ThemeColor::CustomMessageLabel, "[branch]"));
        container.add_child(Box::new(Text::new(label, 1, 0)));
        container.add_child(Box::new(Spacer::new(1)));

        if self.expanded {
            let header = format!("**Branch Summary** ({})\n\n", self.message.branch);
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
            let hint = key_text("ctrl+e");
            let line = format!(
                "{}{}{}",
                t.fg(ThemeColor::CustomMessageText, "Branch summary ("),
                t.fg(ThemeColor::Dim, &hint),
                t.fg(ThemeColor::CustomMessageText, " to expand)"),
            );
            container.add_child(Box::new(Text::new(line, 1, 0)));
        }

        container.render(width)
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapsed_shows_hint() {
        let msg = BranchSummaryMessage {
            branch: "feature/foo".to_string(),
            summary: "Made some changes".to_string(),
        };
        let comp = BranchSummaryMessageComponent::new(msg);
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("expand"));
    }

    #[test]
    fn expanded_shows_summary() {
        let msg = BranchSummaryMessage {
            branch: "feature/foo".to_string(),
            summary: "Made some changes".to_string(),
        };
        let mut comp = BranchSummaryMessageComponent::new(msg);
        comp.set_expanded(true);
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("Made some changes"));
    }
}
