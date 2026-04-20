//! Armin easter egg — animated XBM character.
//!
//! Translated from `components/armin.ts`.
//!
//! A fun Easter egg with animated ASCII/half-block art of Armin.

use tui::tui::Component;

use crate::modes::interactive::theme::{ThemeColor, get_theme};

const WIDTH: usize = 31;
const HEIGHT: usize = 36;

// XBM image: 31x36 pixels, LSB first, 1=background, 0=foreground
#[allow(clippy::all)]
const BITS: &[u8] = &[
    0xff, 0xff, 0xff, 0x7f, 0xff, 0xf0, 0xff, 0x7f, 0xff, 0xed, 0xff, 0x7f, 0xff, 0xdb, 0xff, 0x7f,
    0xff, 0xb7, 0xff, 0x7f, 0xff, 0x77, 0xfe, 0x7f, 0x3f, 0xf8, 0xfe, 0x7f, 0xdf, 0xff, 0xfe, 0x7f,
    0xdf, 0x3f, 0xfc, 0x7f, 0x9f, 0xc3, 0xfb, 0x7f, 0x6f, 0xfc, 0xf4, 0x7f, 0xf7, 0x0f, 0xf7, 0x7f,
    0xf7, 0xff, 0xf7, 0x7f, 0xf7, 0xff, 0xe3, 0x7f, 0xf7, 0x07, 0xe8, 0x7f, 0xef, 0xf8, 0x67, 0x70,
    0x0f, 0xff, 0xbb, 0x6f, 0xf1, 0x00, 0xd0, 0x5b, 0xfd, 0x3f, 0xec, 0x53, 0xc1, 0xff, 0xef, 0x57,
    0x9f, 0xfd, 0xee, 0x5f, 0x9f, 0xfc, 0xae, 0x5f, 0x1f, 0x78, 0xac, 0x5f, 0x3f, 0x00, 0x50, 0x6c,
    0x7f, 0x00, 0xdc, 0x77, 0xff, 0xc0, 0x3f, 0x78, 0xff, 0x01, 0xf8, 0x7f, 0xff, 0x03, 0x9c, 0x78,
    0xff, 0x07, 0x8c, 0x7c, 0xff, 0x0f, 0xce, 0x78, 0xff, 0xff, 0xcf, 0x7f, 0xff, 0xff, 0xcf, 0x78,
    0xff, 0xff, 0xdf, 0x78, 0xff, 0xff, 0xdf, 0x7d, 0xff, 0xff, 0x3f, 0x7e, 0xff, 0xff, 0xff, 0x7f,
];

const BYTES_PER_ROW: usize = (WIDTH + 7) / 8;
const DISPLAY_HEIGHT: usize = (HEIGHT + 1) / 2;

fn get_pixel(x: usize, y: usize) -> bool {
    if y >= HEIGHT || x >= WIDTH {
        return false;
    }
    let byte_index = y * BYTES_PER_ROW + x / 8;
    let bit_index = x % 8;
    if byte_index >= BITS.len() {
        return false;
    }
    ((BITS[byte_index] >> bit_index) & 1) == 0
}

fn get_char(x: usize, row: usize) -> char {
    let upper = get_pixel(x, row * 2);
    let lower = get_pixel(x, row * 2 + 1);
    match (upper, lower) {
        (true, true) => '█',
        (true, false) => '▀',
        (false, true) => '▄',
        (false, false) => ' ',
    }
}

/// Armin easter egg component — renders XBM character art.
pub struct ArminComponent {
    animated: bool,
    frame: usize,
}

impl ArminComponent {
    pub fn new(animated: bool) -> Self {
        Self { animated, frame: 0 }
    }

    pub fn tick(&mut self) {
        if self.animated {
            self.frame = (self.frame + 1) % (DISPLAY_HEIGHT * 3);
        }
    }

    fn render_frame(&self) -> Vec<String> {
        let t = get_theme();
        let mut lines = Vec::new();

        for row in 0..DISPLAY_HEIGHT {
            let mut line = String::new();
            for x in 0..WIDTH {
                let c = get_char(x, row);

                if self.animated {
                    // Wave animation: color shifts down by frame
                    let wave_pos = (row + self.frame) % DISPLAY_HEIGHT;
                    let color = match wave_pos % 6 {
                        0 => ThemeColor::Accent,
                        1 => ThemeColor::Border,
                        2 => ThemeColor::BorderAccent,
                        3 => ThemeColor::Success,
                        4 => ThemeColor::MdHeading,
                        _ => ThemeColor::Muted,
                    };
                    line.push_str(&t.fg(color, &c.to_string()));
                } else {
                    line.push_str(&t.fg(ThemeColor::Accent, &c.to_string()));
                }
            }
            lines.push(line);
        }
        lines
    }
}

impl Component for ArminComponent {
    fn render(&self, _width: u16) -> Vec<String> {
        self.render_frame()
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_correct_height() {
        let comp = ArminComponent::new(false);
        let lines = comp.render(80);
        assert_eq!(lines.len(), DISPLAY_HEIGHT);
    }

    #[test]
    fn animated_tick_advances_frame() {
        let mut comp = ArminComponent::new(true);
        let f0 = comp.frame;
        comp.tick();
        assert_ne!(comp.frame, f0);
    }
}
