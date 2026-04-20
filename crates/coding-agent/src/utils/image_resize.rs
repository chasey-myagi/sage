//! Image resize utilities.
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/image-resize.ts`.
//!
//! Resizes images to fit within configurable max dimensions and encoded-size limits.
//! Uses the `image` crate (via the `photon` wrapper) instead of photon-node WASM.

use base64::Engine as _;

use crate::utils::exif_orientation::apply_exif_orientation;
use crate::utils::photon::{resize, PhotonImage, SamplingFilter};

// ============================================================================
// Public types
// ============================================================================

/// Options for `resize_image`.
#[derive(Debug, Clone)]
pub struct ImageResizeOptions {
    /// Maximum width in pixels. Default: 2000.
    pub max_width: u32,
    /// Maximum height in pixels. Default: 2000.
    pub max_height: u32,
    /// Maximum encoded (base64) size in bytes. Default: ~4.5 MB.
    pub max_bytes: usize,
    /// JPEG quality (0–100). Default: 80.
    pub jpeg_quality: u8,
}

impl Default for ImageResizeOptions {
    fn default() -> Self {
        Self {
            max_width: 2000,
            max_height: 2000,
            // 4.5 MB of base64 payload — provides headroom below Anthropic's 5 MB limit.
            max_bytes: (4.5 * 1024.0 * 1024.0) as usize,
            jpeg_quality: 80,
        }
    }
}

/// Input image for resizing.
#[derive(Debug, Clone)]
pub struct ImageContent {
    /// Base64-encoded image data.
    pub data: String,
    /// MIME type (e.g. `"image/png"`).
    pub mime_type: String,
}

/// Result of a successful resize operation.
#[derive(Debug, Clone)]
pub struct ResizedImage {
    /// Base64-encoded image data.
    pub data: String,
    /// MIME type of the output image.
    pub mime_type: String,
    pub original_width: u32,
    pub original_height: u32,
    pub width: u32,
    pub height: u32,
    /// `true` if any dimension or format change was made.
    pub was_resized: bool,
}

// ============================================================================
// Internal helpers
// ============================================================================

struct EncodedCandidate {
    data: String,
    encoded_size: usize,
    mime_type: String,
}

fn encode_png(image: &PhotonImage) -> Option<EncodedCandidate> {
    let bytes = image.get_bytes().ok()?;
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let encoded_size = data.len();
    Some(EncodedCandidate { data, encoded_size, mime_type: "image/png".to_owned() })
}

fn encode_jpeg(image: &PhotonImage, quality: u8) -> Option<EncodedCandidate> {
    let bytes = image.get_bytes_jpeg(quality).ok()?;
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let encoded_size = data.len();
    Some(EncodedCandidate { data, encoded_size, mime_type: "image/jpeg".to_owned() })
}

/// Try PNG + a range of JPEG qualities at `(width, height)` and return all candidates.
fn try_encodings(image: &PhotonImage, width: u32, height: u32, jpeg_qualities: &[u8]) -> Vec<EncodedCandidate> {
    let resized = resize(image, width, height, SamplingFilter::Lanczos3);
    let mut candidates = Vec::new();
    if let Some(png) = encode_png(&resized) {
        candidates.push(png);
    }
    for &q in jpeg_qualities {
        if let Some(jpg) = encode_jpeg(&resized, q) {
            candidates.push(jpg);
        }
    }
    candidates
}

// ============================================================================
// Public API
// ============================================================================

/// Resize an image to fit within the specified max dimensions and encoded file size.
///
/// Returns `None` if:
/// - the image bytes cannot be decoded
/// - the image cannot be reduced below `max_bytes` even at 1×1
///
/// Strategy (mirrors TypeScript):
/// 1. Apply EXIF orientation.
/// 2. If already within all limits, return immediately (`was_resized: false`).
/// 3. Resize to `max_width`/`max_height` maintaining aspect ratio.
/// 4. Try PNG + JPEG at several qualities; pick the first candidate under `max_bytes`.
/// 5. Progressively reduce dimensions by 25% until the limit is met or 1×1.
pub fn resize_image(img: &ImageContent, options: Option<ImageResizeOptions>) -> Option<ResizedImage> {
    let opts = options.unwrap_or_default();

    let raw = base64::engine::general_purpose::STANDARD.decode(&img.data).ok()?;
    let input_base64_size = img.data.len();

    let raw_image = PhotonImage::new_from_byteslice(&raw).ok()?;
    let image = apply_exif_orientation(raw_image, &raw);

    let original_width = image.get_width();
    let original_height = image.get_height();

    // If already within all limits, return as-is.
    if original_width <= opts.max_width
        && original_height <= opts.max_height
        && input_base64_size < opts.max_bytes
    {
        return Some(ResizedImage {
            data: img.data.clone(),
            mime_type: img.mime_type.clone(),
            original_width,
            original_height,
            width: original_width,
            height: original_height,
            was_resized: false,
        });
    }

    // Calculate initial target dimensions respecting max limits.
    let mut target_width = original_width;
    let mut target_height = original_height;

    if target_width > opts.max_width {
        target_height = (target_height as f64 * opts.max_width as f64 / target_width as f64).round() as u32;
        target_width = opts.max_width;
    }
    if target_height > opts.max_height {
        target_width = (target_width as f64 * opts.max_height as f64 / target_height as f64).round() as u32;
        target_height = opts.max_height;
    }
    target_width = target_width.max(1);
    target_height = target_height.max(1);

    // Deduplicate JPEG quality steps (mirrors TypeScript `Array.from(new Set([...]))`).
    let mut seen = std::collections::HashSet::new();
    let quality_steps: Vec<u8> = [opts.jpeg_quality, 85, 70, 55, 40]
        .iter()
        .copied()
        .filter(|&q| seen.insert(q))
        .collect();

    let mut current_width = target_width;
    let mut current_height = target_height;

    loop {
        let candidates = try_encodings(&image, current_width, current_height, &quality_steps);
        for candidate in candidates {
            if candidate.encoded_size < opts.max_bytes {
                return Some(ResizedImage {
                    data: candidate.data,
                    mime_type: candidate.mime_type,
                    original_width,
                    original_height,
                    width: current_width,
                    height: current_height,
                    was_resized: true,
                });
            }
        }

        if current_width == 1 && current_height == 1 {
            break;
        }

        let next_width = if current_width == 1 { 1 } else { (current_width as f64 * 0.75).floor() as u32 }.max(1);
        let next_height = if current_height == 1 { 1 } else { (current_height as f64 * 0.75).floor() as u32 }.max(1);

        if next_width == current_width && next_height == current_height {
            break;
        }

        current_width = next_width;
        current_height = next_height;
    }

    None
}

/// Format a human-readable note about resized image dimensions.
///
/// Returns `None` when `result.was_resized` is `false`.
///
/// Mirrors `formatDimensionNote()` from TypeScript.
pub fn format_dimension_note(result: &ResizedImage) -> Option<String> {
    if !result.was_resized {
        return None;
    }
    let scale = result.original_width as f64 / result.width as f64;
    Some(format!(
        "[Image: original {}x{}, displayed at {}x{}. Multiply coordinates by {:.2} to map to original image.]",
        result.original_width,
        result.original_height,
        result.width,
        result.height,
        scale,
    ))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png_b64(width: u32, height: u32) -> String {
        let img = image::RgbaImage::from_fn(width, height, |x, _y| {
            image::Rgba([(x % 256) as u8, 100, 150, 255])
        });
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(&buf)
    }

    #[test]
    fn small_image_not_resized() {
        let data = tiny_png_b64(10, 10);
        let img = ImageContent { data, mime_type: "image/png".to_owned() };
        let result = resize_image(&img, None).unwrap();
        assert!(!result.was_resized);
    }

    #[test]
    fn dimension_note_only_when_resized() {
        let data = tiny_png_b64(10, 10);
        let img = ImageContent { data, mime_type: "image/png".to_owned() };
        let result = resize_image(&img, None).unwrap();
        assert!(format_dimension_note(&result).is_none());
    }

    #[test]
    fn oversized_image_gets_resized() {
        let data = tiny_png_b64(3000, 3000);
        let img = ImageContent { data, mime_type: "image/png".to_owned() };
        let result = resize_image(&img, None).unwrap();
        assert!(result.was_resized);
        assert!(result.width <= 2000);
        assert!(result.height <= 2000);
    }

    #[test]
    fn format_dimension_note_contents() {
        let result = ResizedImage {
            data: String::new(),
            mime_type: "image/png".to_owned(),
            original_width: 3000,
            original_height: 1500,
            width: 2000,
            height: 1000,
            was_resized: true,
        };
        let note = format_dimension_note(&result).unwrap();
        assert!(note.contains("3000x1500"));
        assert!(note.contains("2000x1000"));
        assert!(note.contains("1.50"));
    }
}
