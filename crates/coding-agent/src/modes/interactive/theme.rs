//! Theme system for the interactive TUI.
//!
//! Translated from pi-mono `packages/coding-agent/src/modes/interactive/theme/theme.ts`.
//!
//! Handles color definitions, theme loading, and ANSI escape code generation.

use std::collections::HashMap;
use std::env;

// ============================================================================
// Color types
// ============================================================================

/// Color mode — truecolor (24-bit) or 256-color fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Truecolor,
    Color256,
}

/// A color value — either a hex string or a 256-color index.
#[derive(Debug, Clone)]
pub enum ColorValue {
    /// Hex string like `#ff0000` or empty string for terminal default.
    Hex(String),
    /// 256-color palette index (0–255).
    Index(u8),
}

// ============================================================================
// Theme color keys
// ============================================================================

/// All foreground color keys used in the theme.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ThemeColor {
    Accent,
    Border,
    BorderAccent,
    BorderMuted,
    Success,
    Error,
    Warning,
    Muted,
    Dim,
    Text,
    ThinkingText,
    UserMessageText,
    CustomMessageText,
    CustomMessageLabel,
    ToolTitle,
    ToolOutput,
    MdHeading,
    MdLink,
    MdLinkUrl,
    MdCode,
    MdCodeBlock,
    MdCodeBlockBorder,
    MdQuote,
    MdQuoteBorder,
    MdHr,
    MdListBullet,
    ToolDiffAdded,
    ToolDiffRemoved,
    ToolDiffContext,
    SyntaxComment,
    SyntaxKeyword,
    SyntaxFunction,
    SyntaxVariable,
    SyntaxString,
    SyntaxNumber,
    SyntaxType,
    SyntaxOperator,
    SyntaxPunctuation,
    ThinkingOff,
    ThinkingMinimal,
    ThinkingLow,
    ThinkingMedium,
    ThinkingHigh,
    ThinkingXhigh,
    BashMode,
}

/// Background color keys.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ThemeBg {
    SelectedBg,
    UserMessageBg,
    CustomMessageBg,
    ToolPendingBg,
    ToolSuccessBg,
    ToolErrorBg,
    CodeBg,
}

// ============================================================================
// Color detection utilities
// ============================================================================

/// Detect color mode from environment variables (same logic as TS version).
pub fn detect_color_mode() -> ColorMode {
    let colorterm = env::var("COLORTERM").unwrap_or_default();
    if colorterm == "truecolor" || colorterm == "24bit" {
        return ColorMode::Truecolor;
    }
    if env::var("WT_SESSION").is_ok() {
        return ColorMode::Truecolor;
    }
    let term = env::var("TERM").unwrap_or_default();
    if term == "dumb" || term.is_empty() || term == "linux" {
        return ColorMode::Color256;
    }
    if env::var("TERM_PROGRAM").unwrap_or_default() == "Apple_Terminal" {
        return ColorMode::Color256;
    }
    if term == "screen" || term.starts_with("screen-") || term.starts_with("screen.") {
        return ColorMode::Color256;
    }
    ColorMode::Truecolor
}

/// Parse a hex color string to (r, g, b).
pub fn hex_to_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let cleaned = hex.trim_start_matches('#');
    if cleaned.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&cleaned[0..2], 16).ok()?;
    let g = u8::from_str_radix(&cleaned[2..4], 16).ok()?;
    let b = u8::from_str_radix(&cleaned[4..6], 16).ok()?;
    Some((r, g, b))
}

const CUBE_VALUES: [u8; 6] = [0, 95, 135, 175, 215, 255];
const GRAY_COUNT: usize = 24;

fn closest_cube_index(v: u8) -> usize {
    CUBE_VALUES
        .iter()
        .enumerate()
        .min_by_key(|&(_, &cv)| (v as i16 - cv as i16).unsigned_abs())
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn closest_gray_index(gray: u8) -> usize {
    (0..GRAY_COUNT)
        .min_by_key(|&i| {
            let gv = 8u8.wrapping_add((i as u8).wrapping_mul(10));
            (gray as i16 - gv as i16).unsigned_abs()
        })
        .unwrap_or(0)
}

fn color_distance_sq(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> f32 {
    let dr = r1 as f32 - r2 as f32;
    let dg = g1 as f32 - g2 as f32;
    let db = b1 as f32 - b2 as f32;
    dr * dr * 0.299 + dg * dg * 0.587 + db * db * 0.114
}

fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    let ri = closest_cube_index(r);
    let gi = closest_cube_index(g);
    let bi = closest_cube_index(b);
    let cr = CUBE_VALUES[ri];
    let cg = CUBE_VALUES[gi];
    let cb = CUBE_VALUES[bi];
    let cube_idx = 16u8 + (36 * ri + 6 * gi + bi) as u8;
    let cube_dist = color_distance_sq(r, g, b, cr, cg, cb);

    let gray_val = (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32).round() as u8;
    let gi2 = closest_gray_index(gray_val);
    let gv = 8u8.wrapping_add((gi2 as u8).wrapping_mul(10));
    let gray_idx = 232u8 + gi2 as u8;
    let gray_dist = color_distance_sq(r, g, b, gv, gv, gv);

    let max_c = r.max(g).max(b);
    let min_c = r.min(g).min(b);
    let spread = max_c - min_c;

    if spread < 10 && gray_dist < cube_dist {
        gray_idx
    } else {
        cube_idx
    }
}

fn fg_ansi(color: &ColorValue, mode: ColorMode) -> String {
    match color {
        ColorValue::Hex(h) if h.is_empty() => "\x1b[39m".to_string(),
        ColorValue::Hex(h) => {
            if let Some((r, g, b)) = hex_to_rgb(h) {
                match mode {
                    ColorMode::Truecolor => format!("\x1b[38;2;{r};{g};{b}m"),
                    ColorMode::Color256 => {
                        let idx = rgb_to_256(r, g, b);
                        format!("\x1b[38;5;{idx}m")
                    }
                }
            } else {
                "\x1b[39m".to_string()
            }
        }
        ColorValue::Index(i) => format!("\x1b[38;5;{i}m"),
    }
}

fn bg_ansi(color: &ColorValue, mode: ColorMode) -> String {
    match color {
        ColorValue::Hex(h) if h.is_empty() => "\x1b[49m".to_string(),
        ColorValue::Hex(h) => {
            if let Some((r, g, b)) = hex_to_rgb(h) {
                match mode {
                    ColorMode::Truecolor => format!("\x1b[48;2;{r};{g};{b}m"),
                    ColorMode::Color256 => {
                        let idx = rgb_to_256(r, g, b);
                        format!("\x1b[48;5;{idx}m")
                    }
                }
            } else {
                "\x1b[49m".to_string()
            }
        }
        ColorValue::Index(i) => format!("\x1b[48;5;{i}m"),
    }
}

// ============================================================================
// Theme struct
// ============================================================================

/// The compiled theme — pre-computed ANSI escape codes for each color key.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: Option<String>,
    fg_colors: HashMap<ThemeColor, String>,
    bg_colors: HashMap<ThemeBg, String>,
    fg_rgb: HashMap<ThemeColor, (u8, u8, u8)>,
    bg_rgb: HashMap<ThemeBg, (u8, u8, u8)>,
    mode: ColorMode,
}

impl Theme {
    /// Apply foreground color to text, resetting only the foreground after.
    pub fn fg(&self, color: ThemeColor, text: &str) -> String {
        match self.fg_colors.get(&color) {
            Some(ansi) => format!("{ansi}{text}\x1b[39m"),
            None => text.to_string(),
        }
    }

    /// Apply background color to text, resetting only the background after.
    pub fn bg(&self, color: ThemeBg, text: &str) -> String {
        match self.bg_colors.get(&color) {
            Some(ansi) => format!("{ansi}{text}\x1b[49m"),
            None => text.to_string(),
        }
    }

    /// Get the raw foreground ANSI escape for a color.
    pub fn fg_ansi(&self, color: &ThemeColor) -> &str {
        self.fg_colors.get(color).map(|s| s.as_str()).unwrap_or("")
    }

    /// Get the raw background ANSI escape for a color.
    pub fn bg_ansi(&self, color: &ThemeBg) -> &str {
        self.bg_colors.get(color).map(|s| s.as_str()).unwrap_or("")
    }

    pub fn bold(&self, text: &str) -> String {
        format!("\x1b[1m{text}\x1b[22m")
    }

    pub fn italic(&self, text: &str) -> String {
        format!("\x1b[3m{text}\x1b[23m")
    }

    pub fn underline(&self, text: &str) -> String {
        format!("\x1b[4m{text}\x1b[24m")
    }

    pub fn inverse(&self, text: &str) -> String {
        format!("\x1b[7m{text}\x1b[27m")
    }

    pub fn strikethrough(&self, text: &str) -> String {
        format!("\x1b[9m{text}\x1b[29m")
    }

    pub fn color_mode(&self) -> ColorMode {
        self.mode
    }

    /// Get the thinking border color function for a given thinking level.
    pub fn thinking_border_color(&self, level: ThinkingLevel) -> ThemeColor {
        match level {
            ThinkingLevel::Off => ThemeColor::ThinkingOff,
            ThinkingLevel::Minimal => ThemeColor::ThinkingMinimal,
            ThinkingLevel::Low => ThemeColor::ThinkingLow,
            ThinkingLevel::Medium => ThemeColor::ThinkingMedium,
            ThinkingLevel::High => ThemeColor::ThinkingHigh,
            ThinkingLevel::Xhigh => ThemeColor::ThinkingXhigh,
        }
    }

    pub fn bash_mode_border_color(&self) -> ThemeColor {
        ThemeColor::BashMode
    }

    /// Return the ratatui foreground `Color` for a theme color key.
    pub fn ratatui_fg(&self, color: ThemeColor) -> ratatui::style::Color {
        match self.fg_rgb.get(&color) {
            Some(&(r, g, b)) => ratatui::style::Color::Rgb(r, g, b),
            None => ratatui::style::Color::Reset,
        }
    }

    /// Return the ratatui background `Color` for a theme background key.
    pub fn ratatui_bg(&self, color: ThemeBg) -> ratatui::style::Color {
        match self.bg_rgb.get(&color) {
            Some(&(r, g, b)) => ratatui::style::Color::Rgb(r, g, b),
            None => ratatui::style::Color::Reset,
        }
    }
}

/// Thinking level for border coloring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingLevel {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

impl std::str::FromStr for ThinkingLevel {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "off" => Ok(ThinkingLevel::Off),
            "minimal" => Ok(ThinkingLevel::Minimal),
            "low" => Ok(ThinkingLevel::Low),
            "medium" => Ok(ThinkingLevel::Medium),
            "high" => Ok(ThinkingLevel::High),
            "xhigh" => Ok(ThinkingLevel::Xhigh),
            _ => Err(()),
        }
    }
}

// ============================================================================
// Built-in dark theme (from dark.json)
// ============================================================================

/// Build the dark theme using hard-coded color values from dark.json.
pub fn dark_theme(mode: ColorMode) -> Theme {
    let hex = |s: &str| ColorValue::Hex(s.to_string());
    let empty = || ColorValue::Hex(String::new());

    let fg_map: &[(ThemeColor, ColorValue)] = &[
        (ThemeColor::Accent, hex("#8abeb7")),
        (ThemeColor::Border, hex("#5f87ff")),
        (ThemeColor::BorderAccent, hex("#00d7ff")),
        (ThemeColor::BorderMuted, hex("#505050")),
        (ThemeColor::Success, hex("#b5bd68")),
        (ThemeColor::Error, hex("#cc6666")),
        (ThemeColor::Warning, hex("#ffff00")),
        (ThemeColor::Muted, hex("#808080")),
        (ThemeColor::Dim, hex("#666666")),
        (ThemeColor::Text, empty()),
        (ThemeColor::ThinkingText, hex("#808080")),
        (ThemeColor::UserMessageText, empty()),
        (ThemeColor::CustomMessageText, empty()),
        (ThemeColor::CustomMessageLabel, hex("#9575cd")),
        (ThemeColor::ToolTitle, empty()),
        (ThemeColor::ToolOutput, hex("#808080")),
        (ThemeColor::MdHeading, hex("#f0c674")),
        (ThemeColor::MdLink, hex("#81a2be")),
        (ThemeColor::MdLinkUrl, hex("#666666")),
        (ThemeColor::MdCode, hex("#8abeb7")),
        (ThemeColor::MdCodeBlock, hex("#b5bd68")),
        (ThemeColor::MdCodeBlockBorder, hex("#808080")),
        (ThemeColor::MdQuote, hex("#808080")),
        (ThemeColor::MdQuoteBorder, hex("#808080")),
        (ThemeColor::MdHr, hex("#808080")),
        (ThemeColor::MdListBullet, hex("#8abeb7")),
        (ThemeColor::ToolDiffAdded, hex("#b5bd68")),
        (ThemeColor::ToolDiffRemoved, hex("#cc6666")),
        (ThemeColor::ToolDiffContext, hex("#808080")),
        (ThemeColor::SyntaxComment, hex("#6A9955")),
        (ThemeColor::SyntaxKeyword, hex("#569CD6")),
        (ThemeColor::SyntaxFunction, hex("#DCDCAA")),
        (ThemeColor::SyntaxVariable, hex("#9CDCFE")),
        (ThemeColor::SyntaxString, hex("#CE9178")),
        (ThemeColor::SyntaxNumber, hex("#B5CEA8")),
        (ThemeColor::SyntaxType, hex("#4EC9B0")),
        (ThemeColor::SyntaxOperator, hex("#D4D4D4")),
        (ThemeColor::SyntaxPunctuation, hex("#D4D4D4")),
        (ThemeColor::ThinkingOff, hex("#505050")),
        (ThemeColor::ThinkingMinimal, hex("#6e6e6e")),
        (ThemeColor::ThinkingLow, hex("#5f87af")),
        (ThemeColor::ThinkingMedium, hex("#81a2be")),
        (ThemeColor::ThinkingHigh, hex("#b294bb")),
        (ThemeColor::ThinkingXhigh, hex("#d183e8")),
        (ThemeColor::BashMode, hex("#b5bd68")),
    ];

    let bg_map: &[(ThemeBg, ColorValue)] = &[
        (ThemeBg::SelectedBg, hex("#3a3a4a")),
        (ThemeBg::UserMessageBg, hex("#343541")),
        (ThemeBg::CustomMessageBg, hex("#2d2838")),
        (ThemeBg::ToolPendingBg, hex("#282832")),
        (ThemeBg::ToolSuccessBg, hex("#283228")),
        (ThemeBg::ToolErrorBg, hex("#3c2828")),
        (ThemeBg::CodeBg, hex("#2a2a3a")),
    ];

    let mut fg_colors = HashMap::new();
    let mut fg_rgb = HashMap::new();
    for (key, val) in fg_map {
        fg_colors.insert(key.clone(), fg_ansi(val, mode));
        if let ColorValue::Hex(h) = val
            && !h.is_empty()
            && let Some(rgb) = hex_to_rgb(h)
        {
            fg_rgb.insert(key.clone(), rgb);
        }
    }
    let mut bg_colors = HashMap::new();
    let mut bg_rgb = HashMap::new();
    for (key, val) in bg_map {
        bg_colors.insert(key.clone(), bg_ansi(val, mode));
        if let ColorValue::Hex(h) = val
            && let Some(rgb) = hex_to_rgb(h)
        {
            bg_rgb.insert(key.clone(), rgb);
        }
    }

    Theme {
        name: Some("dark".to_string()),
        fg_colors,
        bg_colors,
        fg_rgb,
        bg_rgb,
        mode,
    }
}

/// Build the light theme using hard-coded color values from light.json.
pub fn light_theme(mode: ColorMode) -> Theme {
    let hex = |s: &str| ColorValue::Hex(s.to_string());
    let empty = || ColorValue::Hex(String::new());

    // Light theme has mostly same structure, just different values
    let fg_map: &[(ThemeColor, ColorValue)] = &[
        (ThemeColor::Accent, hex("#005f87")),
        (ThemeColor::Border, hex("#5f87ff")),
        (ThemeColor::BorderAccent, hex("#0087af")),
        (ThemeColor::BorderMuted, hex("#bcbcbc")),
        (ThemeColor::Success, hex("#3a7d44")),
        (ThemeColor::Error, hex("#cc3333")),
        (ThemeColor::Warning, hex("#875f00")),
        (ThemeColor::Muted, hex("#767676")),
        (ThemeColor::Dim, hex("#9e9e9e")),
        (ThemeColor::Text, empty()),
        (ThemeColor::ThinkingText, hex("#767676")),
        (ThemeColor::UserMessageText, empty()),
        (ThemeColor::CustomMessageText, empty()),
        (ThemeColor::CustomMessageLabel, hex("#7b1fa2")),
        (ThemeColor::ToolTitle, empty()),
        (ThemeColor::ToolOutput, hex("#767676")),
        (ThemeColor::MdHeading, hex("#875f00")),
        (ThemeColor::MdLink, hex("#005f87")),
        (ThemeColor::MdLinkUrl, hex("#9e9e9e")),
        (ThemeColor::MdCode, hex("#005f87")),
        (ThemeColor::MdCodeBlock, hex("#3a7d44")),
        (ThemeColor::MdCodeBlockBorder, hex("#767676")),
        (ThemeColor::MdQuote, hex("#767676")),
        (ThemeColor::MdQuoteBorder, hex("#767676")),
        (ThemeColor::MdHr, hex("#767676")),
        (ThemeColor::MdListBullet, hex("#005f87")),
        (ThemeColor::ToolDiffAdded, hex("#3a7d44")),
        (ThemeColor::ToolDiffRemoved, hex("#cc3333")),
        (ThemeColor::ToolDiffContext, hex("#767676")),
        (ThemeColor::SyntaxComment, hex("#6A9955")),
        (ThemeColor::SyntaxKeyword, hex("#0000ff")),
        (ThemeColor::SyntaxFunction, hex("#795e26")),
        (ThemeColor::SyntaxVariable, hex("#001080")),
        (ThemeColor::SyntaxString, hex("#a31515")),
        (ThemeColor::SyntaxNumber, hex("#098658")),
        (ThemeColor::SyntaxType, hex("#267f99")),
        (ThemeColor::SyntaxOperator, hex("#3a3a3a")),
        (ThemeColor::SyntaxPunctuation, hex("#3a3a3a")),
        (ThemeColor::ThinkingOff, hex("#bcbcbc")),
        (ThemeColor::ThinkingMinimal, hex("#9e9e9e")),
        (ThemeColor::ThinkingLow, hex("#5f87af")),
        (ThemeColor::ThinkingMedium, hex("#005f87")),
        (ThemeColor::ThinkingHigh, hex("#6a0dad")),
        (ThemeColor::ThinkingXhigh, hex("#9c27b0")),
        (ThemeColor::BashMode, hex("#3a7d44")),
    ];

    let bg_map: &[(ThemeBg, ColorValue)] = &[
        (ThemeBg::SelectedBg, hex("#e0e0f0")),
        (ThemeBg::UserMessageBg, hex("#e8e8f0")),
        (ThemeBg::CustomMessageBg, hex("#f0e8f8")),
        (ThemeBg::ToolPendingBg, hex("#e8e8f8")),
        (ThemeBg::ToolSuccessBg, hex("#e8f0e8")),
        (ThemeBg::ToolErrorBg, hex("#f8e8e8")),
        (ThemeBg::CodeBg, hex("#ececf4")),
    ];

    let mut fg_colors = HashMap::new();
    let mut fg_rgb = HashMap::new();
    for (key, val) in fg_map {
        fg_colors.insert(key.clone(), fg_ansi(val, mode));
        if let ColorValue::Hex(h) = val
            && !h.is_empty()
            && let Some(rgb) = hex_to_rgb(h)
        {
            fg_rgb.insert(key.clone(), rgb);
        }
    }
    let mut bg_colors = HashMap::new();
    let mut bg_rgb = HashMap::new();
    for (key, val) in bg_map {
        bg_colors.insert(key.clone(), bg_ansi(val, mode));
        if let ColorValue::Hex(h) = val
            && let Some(rgb) = hex_to_rgb(h)
        {
            bg_rgb.insert(key.clone(), rgb);
        }
    }

    Theme {
        name: Some("light".to_string()),
        fg_colors,
        bg_colors,
        fg_rgb,
        bg_rgb,
        mode,
    }
}

// ============================================================================
// Global theme instance
// ============================================================================

use std::sync::{OnceLock, RwLock};

static GLOBAL_THEME: OnceLock<RwLock<Theme>> = OnceLock::new();

/// Initialize the global theme. Call once at startup.
pub fn init_theme(name: Option<&str>) {
    let mode = detect_color_mode();
    let t = match name.unwrap_or("dark") {
        "light" => light_theme(mode),
        _ => dark_theme(mode),
    };
    let lock = GLOBAL_THEME.get_or_init(|| RwLock::new(t.clone()));
    if let Ok(mut w) = lock.write() {
        *w = t;
    }
}

/// Get a clone of the current global theme.
pub fn get_theme() -> Theme {
    let lock = GLOBAL_THEME.get_or_init(|| {
        let mode = detect_color_mode();
        RwLock::new(dark_theme(mode))
    });
    lock.read().map(|g| g.clone()).unwrap_or_else(|_| {
        let mode = detect_color_mode();
        dark_theme(mode)
    })
}

/// Set the global theme by name.
pub fn set_theme(name: &str) -> bool {
    let mode = detect_color_mode();
    let t = match name {
        "light" => light_theme(mode),
        "dark" => dark_theme(mode),
        _ => return false,
    };
    let lock = GLOBAL_THEME.get_or_init(|| RwLock::new(t.clone()));
    if let Ok(mut w) = lock.write() {
        *w = t;
        true
    } else {
        false
    }
}

/// Detect whether terminal has a dark or light background.
pub fn detect_terminal_background() -> &'static str {
    let colorfgbg = env::var("COLORFGBG").unwrap_or_default();
    if !colorfgbg.is_empty() {
        let parts: Vec<&str> = colorfgbg.split(';').collect();
        if let Some(last) = parts.last()
            && let Ok(bg) = last.parse::<u8>()
        {
            return if bg < 8 { "dark" } else { "light" };
        }
    }
    "dark"
}

// ============================================================================
// Language detection helper
// ============================================================================

/// Map a file extension to a language identifier for syntax highlighting.
pub fn language_from_path(file_path: &str) -> Option<&'static str> {
    let ext = file_path.rsplit('.').next()?.to_lowercase();
    match ext.as_str() {
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "py" => Some("python"),
        "rb" => Some("ruby"),
        "rs" => Some("rust"),
        "go" => Some("go"),
        "java" => Some("java"),
        "kt" => Some("kotlin"),
        "swift" => Some("swift"),
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" => Some("cpp"),
        "cs" => Some("csharp"),
        "php" => Some("php"),
        "sh" | "bash" | "zsh" => Some("bash"),
        "ps1" => Some("powershell"),
        "sql" => Some("sql"),
        "html" | "htm" => Some("html"),
        "css" => Some("css"),
        "scss" => Some("scss"),
        "json" => Some("json"),
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        "xml" => Some("xml"),
        "md" | "markdown" => Some("markdown"),
        "lua" => Some("lua"),
        "perl" => Some("perl"),
        "r" => Some("r"),
        "scala" => Some("scala"),
        "hs" => Some("haskell"),
        "graphql" => Some("graphql"),
        "tf" | "hcl" => Some("hcl"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_has_all_fg_colors() {
        let t = dark_theme(ColorMode::Truecolor);
        // Spot-check a few colors
        let accent = t.fg(ThemeColor::Accent, "test");
        assert!(accent.contains("test"));
        assert!(accent.contains("\x1b["));
    }

    #[test]
    fn dark_theme_has_all_bg_colors() {
        let t = dark_theme(ColorMode::Truecolor);
        let user_bg = t.bg(ThemeBg::UserMessageBg, "test");
        assert!(user_bg.contains("test"));
    }

    #[test]
    fn light_theme_builds() {
        let t = light_theme(ColorMode::Truecolor);
        assert_eq!(t.name.as_deref(), Some("light"));
    }

    #[test]
    fn bold_italic_underline() {
        let t = dark_theme(ColorMode::Truecolor);
        assert!(t.bold("x").contains("\x1b[1m"));
        assert!(t.italic("x").contains("\x1b[3m"));
        assert!(t.underline("x").contains("\x1b[4m"));
        assert!(t.inverse("x").contains("\x1b[7m"));
        assert!(t.strikethrough("x").contains("\x1b[9m"));
    }

    #[test]
    fn hex_to_rgb_parses_correctly() {
        let (r, g, b) = hex_to_rgb("#ff0000").unwrap();
        assert_eq!((r, g, b), (255, 0, 0));
    }

    #[test]
    fn language_from_path_works() {
        assert_eq!(language_from_path("foo.rs"), Some("rust"));
        assert_eq!(language_from_path("bar.ts"), Some("typescript"));
        assert_eq!(language_from_path("unknown.xyz"), None);
    }
}
