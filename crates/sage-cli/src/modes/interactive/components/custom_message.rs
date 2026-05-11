//! Custom message component.
//!
//! Translated from `components/custom-message.ts`.
//!
//! Renders extension-provided custom messages.

use tui::components::markdown::{DefaultTextStyle, Markdown, MarkdownTheme};
use tui::components::spacer::Spacer;
use tui::components::text::Text;
use tui::tui::{Component, Container};

use crate::modes::interactive::theme::{ThemeColor, get_theme};

/// A custom message produced by an extension.
#[derive(Debug, Clone)]
pub struct CustomMessage {
    pub custom_type: String,
    pub content: String,
}

/// Renders a custom extension message.
pub struct CustomMessageComponent {
    message: CustomMessage,
    expanded: bool,
}

impl CustomMessageComponent {
    pub fn new(message: CustomMessage) -> Self {
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

impl Component for CustomMessageComponent {
    fn render(&self, width: u16) -> Vec<String> {
        let t = get_theme();
        let mut container = Container::new();

        container.add_child(Box::new(Spacer::new(1)));

        let label = t.bold(&t.fg(
            ThemeColor::CustomMessageLabel,
            &format!("[{}]", self.message.custom_type),
        ));
        container.add_child(Box::new(Text::new(label, 1, 0)));
        container.add_child(Box::new(Spacer::new(1)));

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
            self.message.content.clone(),
            1,
            0,
            build_md_theme(),
            Some(style),
        )));

        container.render(width)
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_label_and_content() {
        let msg = CustomMessage {
            custom_type: "info".to_string(),
            content: "Some extension message".to_string(),
        };
        let comp = CustomMessageComponent::new(msg);
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("[info]"), "Expected label in: {text:?}");
        assert!(text.contains("Some extension message"));
    }
}
