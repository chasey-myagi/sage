//! Footer component — shows pwd, token stats, context usage.
//!
//! Translated from `components/footer.ts`.

use tui::tui::Component;

use crate::modes::interactive::theme::{ThemeColor, get_theme};

// ============================================================================
// Helpers
// ============================================================================

/// Sanitize text for display in a single-line status area.
fn sanitize_status_text(text: &str) -> String {
    text.replace(['\r', '\n', '\t'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Format token count using k/M abbreviations (mirrors TS formatTokens).
pub fn format_tokens(count: u64) -> String {
    if count < 1_000 {
        return count.to_string();
    }
    if count < 10_000 {
        return format!("{:.1}k", count as f64 / 1_000.0);
    }
    if count < 1_000_000 {
        return format!("{}k", count / 1_000);
    }
    if count < 10_000_000 {
        return format!("{:.1}M", count as f64 / 1_000_000.0);
    }
    format!("{}M", count / 1_000_000)
}

// ============================================================================
// Footer data types
// ============================================================================

/// Token/cost usage summary for the footer.
#[derive(Debug, Default, Clone)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: f64,
}

/// Context window usage ratio.
#[derive(Debug, Default, Clone)]
pub struct ContextUsage {
    /// Percentage used (0.0–100.0), or None if unknown.
    pub percent: Option<f64>,
    /// Total context window size in tokens.
    pub context_window: u64,
}

/// Data that the footer component needs to render.
#[derive(Debug, Default, Clone)]
pub struct FooterData {
    pub pwd: String,
    pub git_branch: Option<String>,
    pub session_name: Option<String>,
    pub model_id: Option<String>,
    pub model_provider: Option<String>,
    pub model_has_reasoning: bool,
    pub thinking_level: Option<String>,
    pub token_usage: TokenUsage,
    pub context_usage: ContextUsage,
    pub auto_compact_enabled: bool,
    pub using_subscription: bool,
    pub available_provider_count: usize,
    pub extension_statuses: Vec<(String, String)>,
}

// ============================================================================
// FooterComponent
// ============================================================================

/// Footer component that shows pwd, token stats, and context usage.
pub struct FooterComponent {
    data: FooterData,
}

impl FooterComponent {
    pub fn new(data: FooterData) -> Self {
        Self { data }
    }

    pub fn update_data(&mut self, data: FooterData) {
        self.data = data;
    }
}

impl Component for FooterComponent {
    fn render(&self, width: u16) -> Vec<String> {
        let width = width as usize;
        let t = get_theme();
        let d = &self.data;

        // Build pwd + optional branch + optional session name
        let mut pwd = d.pwd.clone();
        // Replace HOME with ~
        if let Ok(home) = std::env::var("HOME") {
            if pwd.starts_with(&home) {
                pwd = format!("~{}", &pwd[home.len()..]);
            }
        }
        if let Some(branch) = &d.git_branch {
            pwd = format!("{pwd} ({branch})");
        }
        if let Some(name) = &d.session_name {
            pwd = format!("{pwd} • {name}");
        }

        // Build stats left side
        let mut stats_parts = Vec::new();
        if d.token_usage.input > 0 {
            stats_parts.push(format!("↑{}", format_tokens(d.token_usage.input)));
        }
        if d.token_usage.output > 0 {
            stats_parts.push(format!("↓{}", format_tokens(d.token_usage.output)));
        }
        if d.token_usage.cache_read > 0 {
            stats_parts.push(format!("R{}", format_tokens(d.token_usage.cache_read)));
        }
        if d.token_usage.cache_write > 0 {
            stats_parts.push(format!("W{}", format_tokens(d.token_usage.cache_write)));
        }
        if d.token_usage.cost > 0.0 || d.using_subscription {
            let cost_str = if d.using_subscription {
                format!("${:.3} (sub)", d.token_usage.cost)
            } else {
                format!("${:.3}", d.token_usage.cost)
            };
            stats_parts.push(cost_str);
        }

        // Context percentage
        let auto_indicator = if d.auto_compact_enabled {
            " (auto)"
        } else {
            ""
        };
        let context_window_str = format_tokens(d.context_usage.context_window);
        let (context_display, context_pct_val) = match d.context_usage.percent {
            Some(pct) => (
                format!("{:.1}%/{context_window_str}{auto_indicator}", pct),
                pct,
            ),
            None => (format!("?/{context_window_str}{auto_indicator}"), 0.0),
        };

        let context_colored = if context_pct_val > 90.0 {
            t.fg(ThemeColor::Error, &context_display)
        } else if context_pct_val > 70.0 {
            t.fg(ThemeColor::Warning, &context_display)
        } else {
            context_display.clone()
        };
        stats_parts.push(context_colored.clone());

        let stats_left = stats_parts.join(" ");

        // Build right side: model name + optional thinking level
        let model_name = d.model_id.as_deref().unwrap_or("no-model");
        let right_without_provider = if d.model_has_reasoning {
            let level = d.thinking_level.as_deref().unwrap_or("off");
            if level == "off" {
                format!("{model_name} • thinking off")
            } else {
                format!("{model_name} • {level}")
            }
        } else {
            model_name.to_string()
        };

        // Optionally prefix provider
        let right_side = if d.available_provider_count > 1 {
            if let Some(provider) = &d.model_provider {
                format!("({provider}) {right_without_provider}")
            } else {
                right_without_provider.clone()
            }
        } else {
            right_without_provider.clone()
        };

        // Calculate visible widths (approximate — strips ANSI for length calc)
        let stats_left_width = visible_width_approx(&stats_left);
        let right_width = visible_width_approx(&right_side);
        let total_needed = stats_left_width + 2 + right_width;

        let stats_line = if total_needed <= width {
            let pad = " ".repeat(width - stats_left_width - right_width);
            format!("{stats_left}{pad}{right_side}")
        } else {
            // Not enough room for right side
            let available = width.saturating_sub(stats_left_width + 2);
            if available > 0 {
                let truncated: String = right_side.chars().take(available).collect();
                let pad = " ".repeat(
                    width.saturating_sub(stats_left_width + visible_width_approx(&truncated)),
                );
                format!("{stats_left}{pad}{truncated}")
            } else {
                stats_left.clone()
            }
        };

        // Dim both parts of the stats line
        let dim_stats_left = t.fg(ThemeColor::Dim, &stats_left);
        let remainder = &stats_line[stats_left.len()..]; // padding + right_side (no ANSI in stats_left)
        let dim_remainder = t.fg(ThemeColor::Dim, remainder);

        let pwd_line = truncate_to_width(
            &t.fg(ThemeColor::Dim, &pwd),
            width,
            &t.fg(ThemeColor::Dim, "..."),
        );

        let mut lines = vec![pwd_line, format!("{dim_stats_left}{dim_remainder}")];

        // Extension statuses
        if !d.extension_statuses.is_empty() {
            let mut sorted = d.extension_statuses.clone();
            sorted.sort_by(|(a, _), (b, _)| a.cmp(b));
            let status_line = sorted
                .iter()
                .map(|(_, text)| sanitize_status_text(text))
                .collect::<Vec<_>>()
                .join(" ");
            lines.push(truncate_to_width(
                &status_line,
                width,
                &t.fg(ThemeColor::Dim, "..."),
            ));
        }

        lines
    }

    fn invalidate(&mut self) {}
}

/// Approximate visible width by stripping ANSI escape sequences.
fn visible_width_approx(s: &str) -> usize {
    // Simple state machine to strip ESC sequences
    let mut width = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
            continue;
        }
        if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
            continue;
        }
        width += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
    }
    width
}

/// Truncate a string to visible width, appending ellipsis if needed.
fn truncate_to_width(s: &str, max_width: usize, ellipsis: &str) -> String {
    let vis = visible_width_approx(s);
    if vis <= max_width {
        return s.to_string();
    }
    let ellipsis_width = visible_width_approx(ellipsis);
    let target = max_width.saturating_sub(ellipsis_width);
    let mut result = String::new();
    let mut cur_width = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
            result.push(c);
            continue;
        }
        if in_escape {
            result.push(c);
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
            continue;
        }
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if cur_width + cw > target {
            break;
        }
        result.push(c);
        cur_width += cw;
    }
    result.push_str(ellipsis);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_tokens_small() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1_500), "1.5k");
        assert_eq!(format_tokens(15_000), "15k");
        assert_eq!(format_tokens(1_500_000), "1.5M");
    }

    #[test]
    fn sanitize_removes_newlines() {
        let result = sanitize_status_text("hello\nworld\ttab");
        assert_eq!(result, "hello world tab");
    }

    #[test]
    fn footer_renders_two_lines_minimum() {
        let data = FooterData {
            pwd: "/home/user/project".to_string(),
            model_id: Some("claude-3-5-sonnet".to_string()),
            ..Default::default()
        };
        let comp = FooterComponent::new(data);
        let lines = comp.render(80);
        assert!(
            lines.len() >= 2,
            "Expected at least 2 footer lines, got {}",
            lines.len()
        );
    }

    #[test]
    fn footer_shows_git_branch() {
        let data = FooterData {
            pwd: "/home/user/project".to_string(),
            git_branch: Some("main".to_string()),
            ..Default::default()
        };
        let comp = FooterComponent::new(data);
        let lines = comp.render(80);
        // Strip ANSI from first line to check content
        let raw: String = lines[0]
            .chars()
            .filter(|&c| c.is_ascii_graphic() || c == ' ')
            .collect();
        assert!(
            raw.contains("main"),
            "Expected 'main' branch in footer: {raw}"
        );
    }
}
