//! Image format conversion utilities.
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/image-convert.ts`.
//!
//! Converts images to PNG for terminal display (Kitty graphics protocol requires
//! PNG format, `f=100`).

use base64::Engine as _;

use crate::utils::exif_orientation::apply_exif_orientation;
use crate::utils::photon::PhotonImage;

/// Convert image bytes (base64-encoded) to PNG.
///
/// Returns `None` when:
/// - the input is already PNG (no conversion needed would be misleading here,
///   but we still return `Some` with the original data to keep API symmetric)
/// - the image cannot be decoded
///
/// Mirrors `convertToPng()` from TypeScript.
pub fn convert_to_png(
    base64_data: &str,
    mime_type: &str,
) -> Option<ConvertedImage> {
    // Already PNG — no conversion needed.
    if mime_type == "image/png" {
        return Some(ConvertedImage {
            data: base64_data.to_owned(),
            mime_type: "image/png".to_owned(),
        });
    }

    let raw = base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .ok()?;

    let raw_image = PhotonImage::new_from_byteslice(&raw).ok()?;
    let image = apply_exif_orientation(raw_image, &raw);

    let png_bytes = image.get_bytes().ok()?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&png_bytes);

    Some(ConvertedImage {
        data: encoded,
        mime_type: "image/png".to_owned(),
    })
}

/// Result of a successful image conversion.
#[derive(Debug, Clone)]
pub struct ConvertedImage {
    /// Base64-encoded image data.
    pub data: String,
    /// MIME type of the converted image.
    pub mime_type: String,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png_base64() -> String {
        // Build a 1x1 PNG in memory
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut buf),
                image::ImageFormat::Png,
            )
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(&buf)
    }

    #[test]
    fn png_is_returned_as_is() {
        let data = tiny_png_base64();
        let result = convert_to_png(&data, "image/png").unwrap();
        assert_eq!(result.mime_type, "image/png");
        assert_eq!(result.data, data);
    }

    #[test]
    fn invalid_base64_returns_none() {
        let result = convert_to_png("!!!not-base64!!!", "image/jpeg");
        assert!(result.is_none());
    }

    #[test]
    fn invalid_image_bytes_return_none() {
        let garbage = base64::engine::general_purpose::STANDARD.encode(b"not an image");
        let result = convert_to_png(&garbage, "image/jpeg");
        assert!(result.is_none());
    }
}
