/// Terminal image support: Kitty and iTerm2 image protocols.
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

static IMAGE_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageProtocol {
    Kitty,
    ITerm2,
}

#[derive(Debug, Clone)]
pub struct TerminalCapabilities {
    pub images: Option<ImageProtocol>,
    pub true_color: bool,
    pub hyperlinks: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct CellDimensions {
    pub width_px: u32,
    pub height_px: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ImageDimensions {
    pub width_px: u32,
    pub height_px: u32,
}

#[derive(Debug, Clone, Default)]
pub struct ImageRenderOptions {
    pub max_width_cells: Option<u32>,
    pub max_height_cells: Option<u32>,
    pub preserve_aspect_ratio: Option<bool>,
    /// Kitty image ID. If provided, reuses/replaces existing image with this ID.
    pub image_id: Option<u32>,
}

static CACHED_CAPABILITIES: Mutex<Option<TerminalCapabilities>> = Mutex::new(None);
static CELL_DIMENSIONS: Mutex<CellDimensions> = Mutex::new(CellDimensions {
    width_px: 9,
    height_px: 18,
});

pub fn get_cell_dimensions() -> CellDimensions {
    *CELL_DIMENSIONS.lock().unwrap()
}

pub fn set_cell_dimensions(dims: CellDimensions) {
    *CELL_DIMENSIONS.lock().unwrap() = dims;
}

/// Detect terminal capabilities from environment variables.
pub fn detect_capabilities() -> TerminalCapabilities {
    let term_program = std::env::var("TERM_PROGRAM")
        .unwrap_or_default()
        .to_lowercase();
    let term = std::env::var("TERM").unwrap_or_default().to_lowercase();
    let color_term = std::env::var("COLORTERM")
        .unwrap_or_default()
        .to_lowercase();

    if std::env::var("KITTY_WINDOW_ID").is_ok() || term_program == "kitty" {
        return TerminalCapabilities {
            images: Some(ImageProtocol::Kitty),
            true_color: true,
            hyperlinks: true,
        };
    }

    if term_program == "ghostty"
        || term.contains("ghostty")
        || std::env::var("GHOSTTY_RESOURCES_DIR").is_ok()
    {
        return TerminalCapabilities {
            images: Some(ImageProtocol::Kitty),
            true_color: true,
            hyperlinks: true,
        };
    }

    if std::env::var("WEZTERM_PANE").is_ok() || term_program == "wezterm" {
        return TerminalCapabilities {
            images: Some(ImageProtocol::Kitty),
            true_color: true,
            hyperlinks: true,
        };
    }

    if std::env::var("ITERM_SESSION_ID").is_ok() || term_program == "iterm.app" {
        return TerminalCapabilities {
            images: Some(ImageProtocol::ITerm2),
            true_color: true,
            hyperlinks: true,
        };
    }

    if term_program == "vscode" || term_program == "alacritty" {
        return TerminalCapabilities {
            images: None,
            true_color: true,
            hyperlinks: true,
        };
    }

    let true_color = color_term == "truecolor" || color_term == "24bit";
    TerminalCapabilities {
        images: None,
        true_color,
        hyperlinks: true,
    }
}

/// Get cached terminal capabilities.
pub fn get_capabilities() -> TerminalCapabilities {
    let mut lock = CACHED_CAPABILITIES.lock().unwrap();
    if lock.is_none() {
        *lock = Some(detect_capabilities());
    }
    lock.as_ref().unwrap().clone()
}

/// Reset the capabilities cache.
pub fn reset_capabilities_cache() {
    *CACHED_CAPABILITIES.lock().unwrap() = None;
}

const KITTY_PREFIX: &str = "\x1b_G";
const ITERM2_PREFIX: &str = "\x1b]1337;File=";

/// Check if a line contains an image sequence.
pub fn is_image_line(line: &str) -> bool {
    if line.starts_with(KITTY_PREFIX) || line.starts_with(ITERM2_PREFIX) {
        return true;
    }
    line.contains(KITTY_PREFIX) || line.contains(ITERM2_PREFIX)
}

/// Allocate a unique image ID for Kitty graphics protocol.
pub fn allocate_image_id() -> u32 {
    let id = IMAGE_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    if id == 0 { 1 } else { id }
}

/// Encode image data using Kitty graphics protocol.
pub fn encode_kitty(
    base64_data: &str,
    columns: Option<u32>,
    rows: Option<u32>,
    image_id: Option<u32>,
) -> String {
    const CHUNK_SIZE: usize = 4096;

    let mut params: Vec<String> = vec!["a=T".into(), "f=100".into(), "q=2".into()];
    if let Some(c) = columns {
        params.push(format!("c={c}"));
    }
    if let Some(r) = rows {
        params.push(format!("r={r}"));
    }
    if let Some(id) = image_id {
        params.push(format!("i={id}"));
    }

    if base64_data.len() <= CHUNK_SIZE {
        return format!("\x1b_G{};{base64_data}\x1b\\", params.join(","));
    }

    let mut chunks = String::new();
    let mut offset = 0;
    let mut is_first = true;

    while offset < base64_data.len() {
        let end = (offset + CHUNK_SIZE).min(base64_data.len());
        let chunk = &base64_data[offset..end];
        let is_last = end >= base64_data.len();

        if is_first {
            chunks.push_str(&format!("\x1b_G{},m=1;{chunk}\x1b\\", params.join(",")));
            is_first = false;
        } else if is_last {
            chunks.push_str(&format!("\x1b_Gm=0;{chunk}\x1b\\"));
        } else {
            chunks.push_str(&format!("\x1b_Gm=1;{chunk}\x1b\\"));
        }

        offset = end;
    }

    chunks
}

/// Delete a Kitty graphics image by ID.
pub fn delete_kitty_image(image_id: u32) -> String {
    format!("\x1b_Ga=d,d=I,i={image_id}\x1b\\")
}

/// Delete all visible Kitty graphics images.
pub fn delete_all_kitty_images() -> String {
    "\x1b_Ga=d,d=A\x1b\\".to_string()
}

/// Encode image data using iTerm2 inline image protocol.
pub fn encode_iterm2(
    base64_data: &str,
    width: Option<String>,
    height: Option<String>,
    name: Option<&str>,
    preserve_aspect_ratio: Option<bool>,
    inline: bool,
) -> String {
    let mut params: Vec<String> = vec![format!("inline={}", if inline { 1 } else { 0 })];
    if let Some(w) = width {
        params.push(format!("width={w}"));
    }
    if let Some(h) = height {
        params.push(format!("height={h}"));
    }
    if let Some(n) = name {
        let encoded = base64_encode(n.as_bytes());
        params.push(format!("name={encoded}"));
    }
    if preserve_aspect_ratio == Some(false) {
        params.push("preserveAspectRatio=0".into());
    }
    format!("\x1b]1337;File={}:{base64_data}\x07", params.join(";"))
}

fn base64_encode(data: &[u8]) -> String {
    // Simple base64 encoding without external deps
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((n >> 18) & 63) as usize] as char);
        result.push(CHARS[((n >> 12) & 63) as usize] as char);
        result.push(if chunk.len() > 1 {
            CHARS[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            CHARS[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    result
}

/// Calculate number of terminal rows needed for an image at a given width.
pub fn calculate_image_rows(
    image_dimensions: ImageDimensions,
    target_width_cells: u32,
    cell_dimensions: CellDimensions,
) -> u32 {
    let target_width_px = target_width_cells * cell_dimensions.width_px;
    let scale = target_width_px as f64 / image_dimensions.width_px as f64;
    let scaled_height_px = image_dimensions.height_px as f64 * scale;
    let rows = (scaled_height_px / cell_dimensions.height_px as f64).ceil() as u32;
    rows.max(1)
}

/// Parse PNG dimensions from base64-encoded data.
pub fn get_png_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let bytes = decode_base64(base64_data)?;
    if bytes.len() < 24 {
        return None;
    }
    if bytes[0] != 0x89 || bytes[1] != 0x50 || bytes[2] != 0x4e || bytes[3] != 0x47 {
        return None;
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some(ImageDimensions {
        width_px: width,
        height_px: height,
    })
}

/// Parse JPEG dimensions from base64-encoded data.
pub fn get_jpeg_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let bytes = decode_base64(base64_data)?;
    if bytes.len() < 2 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return None;
    }
    let mut offset = 2;
    while offset + 9 < bytes.len() {
        if bytes[offset] != 0xff {
            offset += 1;
            continue;
        }
        let marker = bytes[offset + 1];
        if (0xc0..=0xc2).contains(&marker) {
            if offset + 9 >= bytes.len() {
                return None;
            }
            let height = u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]]) as u32;
            let width = u16::from_be_bytes([bytes[offset + 7], bytes[offset + 8]]) as u32;
            return Some(ImageDimensions {
                width_px: width,
                height_px: height,
            });
        }
        if offset + 3 >= bytes.len() {
            return None;
        }
        let length = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        if length < 2 {
            return None;
        }
        offset += 2 + length;
    }
    None
}

/// Parse GIF dimensions from base64-encoded data.
pub fn get_gif_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let bytes = decode_base64(base64_data)?;
    if bytes.len() < 10 {
        return None;
    }
    let sig = std::str::from_utf8(&bytes[0..6]).ok()?;
    if sig != "GIF87a" && sig != "GIF89a" {
        return None;
    }
    let width = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
    let height = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
    Some(ImageDimensions {
        width_px: width,
        height_px: height,
    })
}

/// Parse WebP dimensions from base64-encoded data.
pub fn get_webp_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let bytes = decode_base64(base64_data)?;
    if bytes.len() < 30 {
        return None;
    }
    let riff = std::str::from_utf8(&bytes[0..4]).ok()?;
    let webp = std::str::from_utf8(&bytes[8..12]).ok()?;
    if riff != "RIFF" || webp != "WEBP" {
        return None;
    }
    let chunk = std::str::from_utf8(&bytes[12..16]).ok()?;
    match chunk {
        "VP8 " => {
            if bytes.len() < 30 {
                return None;
            }
            let width = (u16::from_le_bytes([bytes[26], bytes[27]]) & 0x3fff) as u32;
            let height = (u16::from_le_bytes([bytes[28], bytes[29]]) & 0x3fff) as u32;
            Some(ImageDimensions {
                width_px: width,
                height_px: height,
            })
        }
        "VP8L" => {
            if bytes.len() < 25 {
                return None;
            }
            let bits = u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]);
            let width = (bits & 0x3fff) + 1;
            let height = ((bits >> 14) & 0x3fff) + 1;
            Some(ImageDimensions {
                width_px: width,
                height_px: height,
            })
        }
        "VP8X" => {
            if bytes.len() < 30 {
                return None;
            }
            let width =
                (bytes[24] as u32 | ((bytes[25] as u32) << 8) | ((bytes[26] as u32) << 16)) + 1;
            let height =
                (bytes[27] as u32 | ((bytes[28] as u32) << 8) | ((bytes[29] as u32) << 16)) + 1;
            Some(ImageDimensions {
                width_px: width,
                height_px: height,
            })
        }
        _ => None,
    }
}

/// Get image dimensions by MIME type.
pub fn get_image_dimensions(base64_data: &str, mime_type: &str) -> Option<ImageDimensions> {
    match mime_type {
        "image/png" => get_png_dimensions(base64_data),
        "image/jpeg" => get_jpeg_dimensions(base64_data),
        "image/gif" => get_gif_dimensions(base64_data),
        "image/webp" => get_webp_dimensions(base64_data),
        _ => None,
    }
}

/// Render an image using the appropriate terminal protocol.
pub fn render_image(
    base64_data: &str,
    image_dimensions: ImageDimensions,
    options: &ImageRenderOptions,
) -> Option<(String, u32, Option<u32>)> {
    let caps = get_capabilities();

    match caps.images? {
        ImageProtocol::Kitty => {
            let max_width = options.max_width_cells.unwrap_or(80);
            let rows = calculate_image_rows(image_dimensions, max_width, get_cell_dimensions());
            let sequence = encode_kitty(base64_data, Some(max_width), Some(rows), options.image_id);
            Some((sequence, rows, options.image_id))
        }
        ImageProtocol::ITerm2 => {
            let max_width = options.max_width_cells.unwrap_or(80);
            let rows = calculate_image_rows(image_dimensions, max_width, get_cell_dimensions());
            let sequence = encode_iterm2(
                base64_data,
                Some(max_width.to_string()),
                Some("auto".to_string()),
                None,
                options.preserve_aspect_ratio.or(Some(true)),
                true,
            );
            Some((sequence, rows, None))
        }
    }
}

/// Produce a text fallback for when images can't be rendered.
pub fn image_fallback(
    mime_type: &str,
    dimensions: Option<ImageDimensions>,
    filename: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    if let Some(name) = filename {
        parts.push(name.to_string());
    }
    parts.push(format!("[{mime_type}]"));
    if let Some(d) = dimensions {
        parts.push(format!("{}x{}", d.width_px, d.height_px));
    }
    format!("[Image: {}]", parts.join(" "))
}

fn decode_base64(data: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_image_line_kitty() {
        assert!(is_image_line("\x1b_Ga=T,f=100;data\x1b\\"));
        assert!(!is_image_line("plain text"));
    }

    #[test]
    fn test_is_image_line_iterm2() {
        assert!(is_image_line("\x1b]1337;File=inline=1:data\x07"));
    }

    #[test]
    fn test_calculate_image_rows() {
        let img = ImageDimensions {
            width_px: 100,
            height_px: 100,
        };
        let cell = CellDimensions {
            width_px: 10,
            height_px: 20,
        };
        // target_width_cells=10, target_width_px=100, scale=1.0, scaled_height=100, rows=ceil(100/20)=5
        assert_eq!(calculate_image_rows(img, 10, cell), 5);
    }

    #[test]
    fn test_encode_kitty_small() {
        let seq = encode_kitty("data", Some(80), Some(5), None);
        assert!(seq.starts_with("\x1b_G"));
        assert!(seq.ends_with("\x1b\\"));
    }

    #[test]
    fn test_image_fallback() {
        let s = image_fallback("image/png", None, None);
        assert_eq!(s, "[Image: [image/png]]");
    }

    #[test]
    fn test_image_fallback_with_dims() {
        let dims = ImageDimensions {
            width_px: 800,
            height_px: 600,
        };
        let s = image_fallback("image/jpeg", Some(dims), Some("photo.jpg"));
        assert!(s.contains("photo.jpg"));
        assert!(s.contains("800x600"));
    }

    #[test]
    fn test_delete_kitty_image() {
        let seq = delete_kitty_image(42);
        assert!(seq.contains("i=42"));
    }

    #[test]
    fn test_cell_dimensions() {
        let dims = CellDimensions {
            width_px: 12,
            height_px: 24,
        };
        set_cell_dimensions(dims);
        let got = get_cell_dimensions();
        assert_eq!(got.width_px, 12);
        assert_eq!(got.height_px, 24);
        // Reset to default
        set_cell_dimensions(CellDimensions {
            width_px: 9,
            height_px: 18,
        });
    }

    // =========================================================================
    // Tests from terminal-image.test.ts – isImageLine
    // =========================================================================

    #[test]
    fn test_is_image_line_iterm2_at_start() {
        let line = "\x1b]1337;File=size=100,100;inline=1:base64encodeddata==\x07";
        assert!(is_image_line(line));
    }

    #[test]
    fn test_is_image_line_iterm2_with_text_before() {
        let line = "Some text \x1b]1337;File=size=100,100;inline=1:base64data==\x07 more text";
        assert!(is_image_line(line));
    }

    #[test]
    fn test_is_image_line_iterm2_in_middle() {
        let line = "Text before image...\x1b]1337;File=inline=1:verylongbase64data==...text after";
        assert!(is_image_line(line));
    }

    #[test]
    fn test_is_image_line_iterm2_at_end() {
        let line = "Regular text ending with \x1b]1337;File=inline=1:base64data==\x07";
        assert!(is_image_line(line));
    }

    #[test]
    fn test_is_image_line_iterm2_minimal() {
        let line = "\x1b]1337;File=:\x07";
        assert!(is_image_line(line));
    }

    #[test]
    fn test_is_image_line_kitty_at_start() {
        let line = "\x1b_Ga=T,f=100,t=f,d=base64data...\x1b\\\x1b_Gm=i=1;\x1b\\";
        assert!(is_image_line(line));
    }

    #[test]
    fn test_is_image_line_kitty_with_text_before() {
        let line = "Output: \x1b_Ga=T,f=100;data...\x1b\\\x1b_Gm=i=1;\x1b\\";
        assert!(is_image_line(line));
    }

    #[test]
    fn test_is_image_line_kitty_with_padding() {
        let line = "  \x1b_Ga=T,f=100...\x1b\\\x1b_Gm=i=1;\x1b\\  ";
        assert!(is_image_line(line));
    }

    #[test]
    fn test_is_image_line_long_line_with_image() {
        let image_seq = "\x1b]1337;File=size=800,600;inline=1:";
        let long_line = format!("Text prefix {}{} suffix", image_seq, "A".repeat(300000));
        assert!(long_line.len() > 300000);
        assert!(is_image_line(&long_line));
    }

    #[test]
    fn test_is_image_line_terminal_without_image_support() {
        // Bug fix test: even when "terminal doesn't support images",
        // is_image_line should still return true for lines containing sequences.
        let line =
            "Read image file [image/jpeg]\x1b]1337;File=size=800,600;inline=1:base64data...\x07";
        assert!(is_image_line(line));
    }

    #[test]
    fn test_is_image_line_with_ansi_codes_before() {
        let line = "\x1b[31mError output \x1b]1337;File=inline=1:image==\x07";
        assert!(is_image_line(line));
    }

    #[test]
    fn test_is_image_line_with_ansi_codes_after() {
        let line = "\x1b_Ga=T,f=100:data...\x1b\\\x1b_Gm=i=1;\x1b\\\x1b[0m reset";
        assert!(is_image_line(line));
    }

    #[test]
    fn test_is_not_image_line_plain_text() {
        let line = "This is just a regular text line without any escape sequences";
        assert!(!is_image_line(line));
    }

    #[test]
    fn test_is_not_image_line_only_ansi_codes() {
        let line = "\x1b[31mRed text\x1b[0m and \x1b[32mgreen text\x1b[0m";
        assert!(!is_image_line(line));
    }

    #[test]
    fn test_is_not_image_line_cursor_movement_codes() {
        let line = "\x1b[1A\x1b[2KLine cleared and moved up";
        assert!(!is_image_line(line));
    }

    #[test]
    fn test_is_not_image_line_partial_iterm2_sequence() {
        let line = "Some text with ]1337;File but missing ESC at start";
        assert!(!is_image_line(line));
    }

    #[test]
    fn test_is_not_image_line_partial_kitty_sequence() {
        let line = "Some text with _G but missing ESC at start";
        assert!(!is_image_line(line));
    }

    #[test]
    fn test_is_not_image_line_empty() {
        assert!(!is_image_line(""));
    }

    #[test]
    fn test_is_not_image_line_newline_only() {
        assert!(!is_image_line("\n"));
        assert!(!is_image_line("\n\n"));
    }

    #[test]
    fn test_is_image_line_mixed_kitty_and_iterm2() {
        let line = "Kitty: \x1b_Ga=T...\x1b\\\x1b_Gm=i=1;\x1b\\ iTerm2: \x1b]1337;File=inline=1:data==\x07";
        assert!(is_image_line(line));
    }

    #[test]
    fn test_is_image_line_multiple_segments() {
        let line = "Start \x1b]1337;File=img1==\x07 middle \x1b]1337;File=img2==\x07 end";
        assert!(is_image_line(line));
    }

    #[test]
    fn test_is_not_image_line_file_path_with_keywords() {
        let line = "/path/to/File_1337_backup/image.jpg";
        assert!(!is_image_line(line));
    }

    // =========================================================================
    // Tests from bug-regression-isimageline-startswith-bug.test.ts
    // =========================================================================

    #[test]
    fn test_is_image_line_kitty_in_any_position() {
        let scenarios = vec![
            "At start: \x1b_Ga=T,f=100,data...\x1b\\".to_string(),
            format!("Prefix \x1b_Ga=T,data...\x1b\\"),
            format!("Suffix text \x1b_Ga=T,data...\x1b\\ suffix"),
            format!("Middle \x1b_Ga=T,data...\x1b\\ more text"),
            format!("Text before \x1b_Ga=T,f=100{} text after", "A".repeat(3000)),
        ];
        for line in &scenarios {
            assert!(
                is_image_line(line),
                "Should detect Kitty sequence: {:.50}",
                line
            );
        }
    }

    #[test]
    fn test_is_image_line_iterm2_in_any_position() {
        let scenarios = vec![
            "At start: \x1b]1337;File=size=100,100:base64...\x07".to_string(),
            "Prefix \x1b]1337;File=inline=1:data==\x07".to_string(),
            "Suffix text \x1b]1337;File=inline=1:data==\x07 suffix".to_string(),
            "Middle \x1b]1337;File=inline=1:data==\x07 more text".to_string(),
            format!(
                "Text before \x1b]1337;File=size=800,600;inline=1:{} text after",
                "B".repeat(3000)
            ),
        ];
        for line in &scenarios {
            assert!(
                is_image_line(line),
                "Should detect iTerm2 sequence: {:.50}",
                line
            );
        }
    }

    #[test]
    fn test_is_image_line_tool_output() {
        let line =
            "Read image file [image/jpeg]\x1b]1337;File=size=800,600;inline=1:base64image...\x07";
        assert!(is_image_line(line));
    }

    #[test]
    fn test_is_image_line_very_long_with_iterm2() {
        // A very long line (300KB+) containing an iTerm2 escape sequence should be detected.
        let iterm2_seq = "\x1b]1337;File=size=800,600;inline=1:";
        // Need enough "A"s to push total length over 300000
        // iterm2_seq is ~36 chars, prefix "Output: " is 8, suffix " end of output" is 15
        // So we need 300000 - 8 - 36 - 15 = 299941 "A"s
        let crash_line = format!("Output: {}{} end of output", iterm2_seq, "A".repeat(300000));
        assert!(
            crash_line.len() > 300000,
            "line should be > 300000 chars, got {}",
            crash_line.len()
        );
        assert!(is_image_line(&crash_line));
    }

    #[test]
    fn test_is_not_image_line_plain_long_text() {
        let long_text = "A".repeat(100000);
        assert!(!is_image_line(&long_text));
    }

    #[test]
    fn test_is_not_image_line_file_paths() {
        let paths = vec![
            "/path/to/1337/image.jpg",
            "/usr/local/bin/File_converter",
            "~/Documents/1337File_backup.png",
            "./_G_test_file.txt",
        ];
        for path in &paths {
            assert!(
                !is_image_line(path),
                "Should not falsely detect image in path: {}",
                path
            );
        }
    }
}
