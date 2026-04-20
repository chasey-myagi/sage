//! User message component.
//!
//! Translated from `components/user-message.ts`.
//!
//! Renders a user turn in the conversation with distinctive background styling
//! and OSC 133 shell integration markers.

use tui::tui::{Component, Container};
use tui::components::spacer::Spacer;
use tui::components::markdown::{Markdown, DefaultTextStyle, MarkdownTheme};

use crate::modes::interactive::theme::{get_theme, ThemeColor, ThemeBg};

// OSC 133 semantic shell integration markers.
const OSC133_ZONE_START: &str = "\x1b]133;A\x07";
const OSC133_ZONE_END: &str = "\x1b]133;B\x07";
const OSC133_ZONE_FINAL: &str = "\x1b]133;C\x07";

/// Renders a user turn with background highlight and OSC 133 markers.
pub struct UserMessageComponent {
    container: Container,
}

impl UserMessageComponent {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let t = get_theme();

        let md_theme = default_markdown_theme();
        let default_style = DefaultTextStyle {
            color: Some(Box::new({
                let t2 = t.clone();
                move |s: &str| t2.fg(ThemeColor::UserMessageText, s)
            })),
            bg_color: Some(Box::new({
                let t3 = t.clone();
                move |s: &str| t3.bg(ThemeBg::UserMessageBg, s)
            })),
            bold: false,
            italic: false,
            strikethrough: false,
            underline: false,
        };

        let mut container = Container::new();
        container.add_child(Box::new(Spacer::new(1)));
        container.add_child(Box::new(Markdown::new(text, 1, 1, md_theme, Some(default_style))));

        Self { container }
    }
}

fn default_markdown_theme() -> MarkdownTheme {
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

impl Component for UserMessageComponent {
    fn render(&self, width: u16) -> Vec<String> {
        let mut lines = self.container.render(width);
        if lines.is_empty() {
            return lines;
        }
        // Wrap with OSC 133 shell integration markers
        lines[0] = format!("{OSC133_ZONE_START}{}", lines[0]);
        let last = lines.len() - 1;
        lines[last] = format!("{}{OSC133_ZONE_END}{OSC133_ZONE_FINAL}", lines[last]);
        lines
    }

    fn invalidate(&mut self) {
        self.container.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_osc133_markers() {
        let comp = UserMessageComponent::new("Hello world");
        let lines = comp.render(80);
        assert!(!lines.is_empty());
        // First non-empty line should have OSC 133 start marker
        let has_start = lines.iter().any(|l| l.contains(OSC133_ZONE_START));
        assert!(has_start, "Expected OSC 133 start marker in rendered output");
        let has_end = lines.iter().any(|l| l.contains(OSC133_ZONE_END));
        assert!(has_end, "Expected OSC 133 end marker in rendered output");
    }
}
