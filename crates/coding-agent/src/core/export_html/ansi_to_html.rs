//! ANSI escape code to HTML converter.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/export-html/ansi-to-html.ts`.
//!
//! Converts terminal ANSI colour/style codes to HTML with inline styles.
//!
//! # Supported escape codes
//!
//! - Standard foreground (30-37) and bright (90-97)
//! - Standard background (40-47) and bright (100-107)
//! - 256-colour palette (`38;5;N` / `48;5;N`)
//! - RGB true colour (`38;2;R;G;B` / `48;2;R;G;B`)
//! - Text styles: bold (1), dim (2), italic (3), underline (4)
//! - Reset (0)

// ============================================================================
// Colour palette
// ============================================================================

/// Standard ANSI colour palette (indices 0-15).
const ANSI_COLORS: [&str; 16] = [
    "#000000", // 0: black
    "#800000", // 1: red
    "#008000", // 2: green
    "#808000", // 3: yellow
    "#000080", // 4: blue
    "#800080", // 5: magenta
    "#008080", // 6: cyan
    "#c0c0c0", // 7: white
    "#808080", // 8: bright black
    "#ff0000", // 9: bright red
    "#00ff00", // 10: bright green
    "#ffff00", // 11: bright yellow
    "#0000ff", // 12: bright blue
    "#ff00ff", // 13: bright magenta
    "#00ffff", // 14: bright cyan
    "#ffffff", // 15: bright white
];

// ============================================================================
// Colour helpers
// ============================================================================

/// Convert a 256-colour palette index to a hex colour string.
fn color256_to_hex(index: u8) -> String {
    let idx = index as usize;
    // Standard colours (0-15)
    if idx < 16 {
        return ANSI_COLORS[idx].to_string();
    }
    // Colour cube (16-231): 6×6×6 = 216 colours
    if idx < 232 {
        let cube = idx - 16;
        let r = cube / 36;
        let g = (cube % 36) / 6;
        let b = cube % 6;
        let to_component = |n: usize| if n == 0 { 0u8 } else { 55 + n as u8 * 40 };
        return format!(
            "#{:02x}{:02x}{:02x}",
            to_component(r),
            to_component(g),
            to_component(b)
        );
    }
    // Grayscale (232-255)
    let gray = 8u8 + (index - 232) * 10;
    format!("#{gray:02x}{gray:02x}{gray:02x}")
}

// ============================================================================
// HTML escaping
// ============================================================================

/// Escape HTML special characters.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#039;"),
            c => out.push(c),
        }
    }
    out
}

// ============================================================================
// Style state
// ============================================================================

#[derive(Debug, Clone, Default)]
struct TextStyle {
    fg: Option<String>,
    bg: Option<String>,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
}

impl TextStyle {
    fn has_style(&self) -> bool {
        self.fg.is_some()
            || self.bg.is_some()
            || self.bold
            || self.dim
            || self.italic
            || self.underline
    }

    fn to_inline_css(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(ref fg) = self.fg {
            parts.push(format!("color:{fg}"));
        }
        if let Some(ref bg) = self.bg {
            parts.push(format!("background-color:{bg}"));
        }
        if self.bold {
            parts.push("font-weight:bold".to_string());
        }
        if self.dim {
            parts.push("opacity:0.6".to_string());
        }
        if self.italic {
            parts.push("font-style:italic".to_string());
        }
        if self.underline {
            parts.push("text-decoration:underline".to_string());
        }
        parts.join(";")
    }

    /// Apply a sequence of SGR parameters to this style.
    ///
    /// Mirrors pi-mono `applySgrCode`.
    fn apply_sgr(&mut self, params: &[u32]) {
        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => {
                    *self = TextStyle::default();
                }
                1 => self.bold = true,
                2 => self.dim = true,
                3 => self.italic = true,
                4 => self.underline = true,
                22 => {
                    self.bold = false;
                    self.dim = false;
                }
                23 => self.italic = false,
                24 => self.underline = false,
                // Standard foreground colours
                c @ 30..=37 => {
                    self.fg = Some(ANSI_COLORS[(c - 30) as usize].to_string());
                }
                // Extended foreground colour
                38 if params.len() > i + 1 => {
                    match params[i + 1] {
                        5 if params.len() > i + 2 => {
                            self.fg = Some(color256_to_hex(params[i + 2] as u8));
                            i += 2;
                        }
                        2 if params.len() > i + 4 => {
                            let r = params[i + 2];
                            let g = params[i + 3];
                            let b = params[i + 4];
                            self.fg = Some(format!("rgb({r},{g},{b})"));
                            i += 4;
                        }
                        _ => {}
                    }
                }
                39 => self.fg = None,
                // Standard background colours
                c @ 40..=47 => {
                    self.bg = Some(ANSI_COLORS[(c - 40) as usize].to_string());
                }
                // Extended background colour
                48 if params.len() > i + 1 => {
                    match params[i + 1] {
                        5 if params.len() > i + 2 => {
                            self.bg = Some(color256_to_hex(params[i + 2] as u8));
                            i += 2;
                        }
                        2 if params.len() > i + 4 => {
                            let r = params[i + 2];
                            let g = params[i + 3];
                            let b = params[i + 4];
                            self.bg = Some(format!("rgb({r},{g},{b})"));
                            i += 4;
                        }
                        _ => {}
                    }
                }
                49 => self.bg = None,
                // Bright foreground colours
                c @ 90..=97 => {
                    self.fg = Some(ANSI_COLORS[(c - 90 + 8) as usize].to_string());
                }
                // Bright background colours
                c @ 100..=107 => {
                    self.bg = Some(ANSI_COLORS[(c - 100 + 8) as usize].to_string());
                }
                _ => {} // Ignore unrecognised codes
            }
            i += 1;
        }
    }
}

// ============================================================================
// ANSI → HTML conversion
// ============================================================================

/// Convert ANSI-escaped text to HTML with inline styles.
///
/// Mirrors pi-mono `ansiToHtml`.
pub fn ansi_to_html(text: &str) -> String {
    // Simple state machine that scans for ESC[ ... m sequences.
    let mut style = TextStyle::default();
    let mut result = String::new();
    let mut in_span = false;

    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Look for ESC[
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // Find the terminating 'm'
            let start = i + 2;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'm' {
                end += 1;
            }
            if end < bytes.len() && bytes[end] == b'm' {
                // Parse SGR params
                let param_str = &text[start..end];
                let params: Vec<u32> = if param_str.is_empty() {
                    vec![0]
                } else {
                    param_str
                        .split(';')
                        .map(|p| p.parse::<u32>().unwrap_or(0))
                        .collect()
                };

                // Close existing span
                if in_span {
                    result.push_str("</span>");
                    in_span = false;
                }

                style.apply_sgr(&params);

                if style.has_style() {
                    result.push_str(&format!("<span style=\"{}\">", style.to_inline_css()));
                    in_span = true;
                }

                i = end + 1;
                continue;
            }
        }

        // Regular character
        let ch = text[i..].chars().next().unwrap_or('\0');
        let encoded = escape_html(&ch.to_string());
        result.push_str(&encoded);
        i += ch.len_utf8();
    }

    if in_span {
        result.push_str("</span>");
    }

    result
}

/// Convert a slice of ANSI-escaped lines to HTML.
///
/// Each line is wrapped in a `<div class="ansi-line">` element.
///
/// Mirrors pi-mono `ansiLinesToHtml`.
pub fn ansi_lines_to_html(lines: &[&str]) -> String {
    lines
        .iter()
        .map(|line| {
            let html = ansi_to_html(line);
            let inner = if html.is_empty() {
                "&nbsp;".to_string()
            } else {
                html
            };
            format!("<div class=\"ansi-line\">{inner}</div>")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_unchanged() {
        assert_eq!(ansi_to_html("hello world"), "hello world");
    }

    #[test]
    fn html_special_chars_escaped() {
        assert_eq!(ansi_to_html("<b>&amp;</b>"), "&lt;b&gt;&amp;amp;&lt;/b&gt;");
    }

    #[test]
    fn bold_code() {
        let input = "\x1b[1mhello\x1b[0m";
        let out = ansi_to_html(input);
        assert!(out.contains("font-weight:bold"), "should add bold style");
        assert!(out.contains("hello"));
        assert!(out.contains("</span>"));
    }

    #[test]
    fn foreground_colour() {
        let input = "\x1b[31mred\x1b[0m";
        let out = ansi_to_html(input);
        assert!(out.contains("color:#800000"), "should have red fg");
        assert!(out.contains("red"));
    }

    #[test]
    fn background_colour() {
        let input = "\x1b[42mgreen bg\x1b[0m";
        let out = ansi_to_html(input);
        assert!(out.contains("background-color:#008000"));
    }

    #[test]
    fn bright_foreground() {
        let input = "\x1b[91mbright red\x1b[0m";
        let out = ansi_to_html(input);
        assert!(out.contains("color:#ff0000"));
    }

    #[test]
    fn reset_closes_span() {
        let input = "\x1b[1mbold\x1b[0m plain";
        let out = ansi_to_html(input);
        assert!(out.contains("</span>"));
        assert!(out.ends_with(" plain"));
    }

    #[test]
    fn color256_standard() {
        assert_eq!(color256_to_hex(0), "#000000");
        assert_eq!(color256_to_hex(15), "#ffffff");
    }

    #[test]
    fn color256_cube() {
        // index 16 = r=0,g=0,b=0 in cube → (0,0,0) → #000000
        assert_eq!(color256_to_hex(16), "#000000");
        // index 17 = r=0,g=0,b=1 → b = 55+40 = 95
        assert_eq!(color256_to_hex(17), "#00005f");
    }

    #[test]
    fn color256_grayscale() {
        // index 232 → gray = 8
        assert_eq!(color256_to_hex(232), "#080808");
    }

    #[test]
    fn ansi_256_foreground() {
        // 38;5;196 = 256-colour red
        let input = "\x1b[38;5;196mtest\x1b[0m";
        let out = ansi_to_html(input);
        assert!(out.contains("color:"), "should have fg colour");
    }

    #[test]
    fn ansi_rgb_foreground() {
        let input = "\x1b[38;2;255;128;0morange\x1b[0m";
        let out = ansi_to_html(input);
        assert!(out.contains("color:rgb(255,128,0)"));
    }

    #[test]
    fn ansi_lines_to_html_wraps_divs() {
        let lines = vec!["line 1", "line 2"];
        let html = ansi_lines_to_html(&lines);
        assert!(html.contains("<div class=\"ansi-line\">line 1</div>"));
        assert!(html.contains("<div class=\"ansi-line\">line 2</div>"));
    }

    #[test]
    fn ansi_lines_to_html_empty_line_uses_nbsp() {
        let lines = vec![""];
        let html = ansi_lines_to_html(&lines);
        assert!(html.contains("&nbsp;"));
    }

    #[test]
    fn italic_underline_dim() {
        let input = "\x1b[2;3;4mstyle\x1b[0m";
        let out = ansi_to_html(input);
        assert!(out.contains("opacity:0.6"));
        assert!(out.contains("font-style:italic"));
        assert!(out.contains("text-decoration:underline"));
    }
}
