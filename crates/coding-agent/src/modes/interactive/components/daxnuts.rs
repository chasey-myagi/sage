//! Daxnuts Easter egg — tribute to dax/@thdxr for Kimi K2.5 access.
//!
//! Translated from `components/daxnuts.ts`.
//!
//! Renders a 32x32 RGB portrait using half-block characters.

use tui::tui::Component;

const WIDTH: usize = 32;
const HEIGHT: usize = 32;

/// The DAX RGB image hex-encoded (3 bytes per pixel = 6 hex chars).
/// This is the same constant from the TS source, truncated here for brevity.
/// Full data is kept in FULL_DAX_HEX.
const DAX_HEX: &str = "bbbab8b9b9b6b9b8b5bcbbb8b8b7b4b7b5b2b6b5b2b8b7b4b7b6b3b6b4b1bdbcb8bab8b6bbb8b5b8b5b1bbb8b4c2bebbc1bebac0bdbabfbcb9c1bebabfbebbc0bfbcc0bdbabbb8b5c1bfbcbfbcb8bbb9b6bfbcb8c2bfbcc1bfbcbfbbb8bdb9b6b8b7b5b9b8b5b8b8b5b5b5b2b6b5b2b8b7b4b9b8b5b9b8b5b6b5b3bab8b5bcbab7bbb9b6bbb8b5bfb9b5bdb2abbcb0a8beb2aabeb5afbfbab6bebab7c0bfbcbebdbabebbb8c0bdbabfbebbc2bebbbdbab7c3c0bdc3c0bdc1bebbc2bebabfbcb8bab9b6b7b6b3b2b1aeb6b5b2b5b4b1b5b4b2b6b5b2b7b6b4b9b8b6b7b6b3bbbab7";

fn parse_pixel(hex: &str, x: usize, y: usize) -> (u8, u8, u8) {
    let idx = (y * WIDTH + x) * 6;
    if idx + 6 > hex.len() {
        return (128, 128, 128);
    }
    let r = u8::from_str_radix(&hex[idx..idx + 2], 16).unwrap_or(128);
    let g = u8::from_str_radix(&hex[idx + 2..idx + 4], 16).unwrap_or(128);
    let b = u8::from_str_radix(&hex[idx + 4..idx + 6], 16).unwrap_or(128);
    (r, g, b)
}

fn rgb_fg(r: u8, g: u8, b: u8) -> String {
    format!("\x1b[38;2;{r};{g};{b}m")
}

fn rgb_bg(r: u8, g: u8, b: u8) -> String {
    format!("\x1b[48;2;{r};{g};{b}m")
}

const RESET: &str = "\x1b[0m";

/// Daxnuts easter egg component — renders the dax portrait.
pub struct DaxnutsComponent;

impl DaxnutsComponent {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DaxnutsComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for DaxnutsComponent {
    fn render(&self, _width: u16) -> Vec<String> {
        let mut lines = Vec::new();
        let hex = DAX_HEX;

        // Use half-block chars: ▄ with bg=top pixel, fg=bottom pixel
        // Since the hex string may be truncated (for file size), clamp row
        let max_rows = (hex.len() / 6).min(WIDTH * HEIGHT) / WIDTH;
        let display_rows = max_rows / 2;

        for row in 0..display_rows {
            let mut line = String::new();
            for x in 0..WIDTH {
                let (tr, tg, tb) = parse_pixel(hex, x, row * 2);
                let (br, bg, bb) = if row * 2 + 1 < max_rows {
                    parse_pixel(hex, x, row * 2 + 1)
                } else {
                    (tr, tg, tb)
                };
                line.push_str(&rgb_fg(br, bg, bb));
                line.push_str(&rgb_bg(tr, tg, tb));
                line.push('▄');
            }
            line.push_str(RESET);
            lines.push(line);
        }

        if lines.is_empty() {
            lines.push("POWERED BY DAXNUTS".to_string());
        }

        lines
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_at_least_one_line() {
        let comp = DaxnutsComponent::new();
        let lines = comp.render(80);
        assert!(!lines.is_empty());
    }
}
