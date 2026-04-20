//! Skill invocation message component.
//!
//! Translated from `components/skill-invocation-message.ts`.

use tui::tui::{Component, Container};
use tui::components::text::Text;
use tui::components::markdown::{Markdown, MarkdownTheme, DefaultTextStyle};

use crate::modes::interactive::theme::{get_theme, ThemeColor};
use crate::modes::interactive::components::keybinding_hints::key_text;

/// A parsed skill block found in user input.
#[derive(Debug, Clone)]
pub struct ParsedSkillBlock {
    pub name: String,
    pub content: String,
}

/// Renders a skill invocation with collapsed/expanded state.
pub struct SkillInvocationMessageComponent {
    skill_block: ParsedSkillBlock,
    expanded: bool,
}

impl SkillInvocationMessageComponent {
    pub fn new(skill_block: ParsedSkillBlock) -> Self {
        Self { skill_block, expanded: false }
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

impl Component for SkillInvocationMessageComponent {
    fn render(&self, width: u16) -> Vec<String> {
        let t = get_theme();
        let mut container = Container::new();

        if self.expanded {
            let label = t.bold(&t.fg(ThemeColor::CustomMessageLabel, "[skill]"));
            container.add_child(Box::new(Text::new(label, 1, 0)));

            let header = format!("**{}**\n\n", self.skill_block.name);
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
                format!("{}{}", header, self.skill_block.content),
                1,
                0,
                build_md_theme(),
                Some(style),
            )));
        } else {
            let hint = key_text("ctrl+e");
            let line = format!(
                "{} {} {}",
                t.bold(&t.fg(ThemeColor::CustomMessageLabel, "[skill]")),
                t.fg(ThemeColor::CustomMessageText, &self.skill_block.name),
                t.fg(ThemeColor::Dim, &format!("({hint} to expand)")),
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
    fn collapsed_shows_skill_name() {
        let sb = ParsedSkillBlock {
            name: "code-review".to_string(),
            content: "Review the code carefully.".to_string(),
        };
        let comp = SkillInvocationMessageComponent::new(sb);
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("code-review"));
        assert!(text.contains("[skill]"));
    }

    #[test]
    fn expanded_shows_content() {
        let sb = ParsedSkillBlock {
            name: "code-review".to_string(),
            content: "Review the code carefully.".to_string(),
        };
        let mut comp = SkillInvocationMessageComponent::new(sb);
        comp.set_expanded(true);
        let lines = comp.render(80);
        let text = lines.join("\n");
        assert!(text.contains("Review the code carefully"));
    }
}
