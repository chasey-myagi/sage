//! Image processing wrapper using the `image` crate.
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/photon.ts`.
//!
//! The TypeScript original wraps `@silvia-odwyer/photon-node` (a Rust/WASM package).
//! Here we use the native `image` crate directly, which provides equivalent functionality
//! without WASM overhead.

use image::{DynamicImage, ImageFormat};

// ============================================================================
// Public types mirroring photon's PhotonImage interface
// ============================================================================

/// A decoded image with raw RGBA pixel data.
///
/// Mirrors `PhotonImage` from `@silvia-odwyer/photon-node`.
pub struct PhotonImage {
    inner: DynamicImage,
}

impl PhotonImage {
    /// Decode image bytes into a `PhotonImage`.
    pub fn new_from_byteslice(bytes: &[u8]) -> anyhow::Result<Self> {
        let inner = image::load_from_memory(bytes)?;
        Ok(Self { inner })
    }

    /// Width in pixels.
    pub fn get_width(&self) -> u32 {
        self.inner.width()
    }

    /// Height in pixels.
    pub fn get_height(&self) -> u32 {
        self.inner.height()
    }

    /// Encode to PNG bytes.
    pub fn get_bytes(&self) -> anyhow::Result<Vec<u8>> {
        encode_image(&self.inner, ImageFormat::Png)
    }

    /// Encode to JPEG bytes at the given quality (0–100).
    pub fn get_bytes_jpeg(&self, quality: u8) -> anyhow::Result<Vec<u8>> {
        let rgb = self.inner.to_rgb8();
        let mut buf = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
        encoder.encode_image(&rgb)?;
        Ok(buf)
    }

    /// Raw RGBA pixel bytes (row-major, RGBA).
    pub fn get_raw_pixels(&self) -> Vec<u8> {
        self.inner.to_rgba8().into_raw()
    }

    /// Construct from raw RGBA pixels with explicit dimensions.
    ///
    /// Mirrors `new PhotonImage(dst, height, width)` — note that photon's
    /// constructor takes (raw_pixels, width, height) despite the TypeScript
    /// call passing (dst, newH, newW) after a rotate where the dimensions swap.
    pub fn from_raw_rgba(pixels: Vec<u8>, width: u32, height: u32) -> Self {
        let buf = image::RgbaImage::from_raw(width, height, pixels)
            .expect("pixel buffer size mismatch");
        Self { inner: DynamicImage::ImageRgba8(buf) }
    }

    pub fn inner(&self) -> &DynamicImage {
        &self.inner
    }

    pub fn into_inner(self) -> DynamicImage {
        self.inner
    }
}

// ============================================================================
// Resize
// ============================================================================

/// Sampling filter — mirrors `photon.SamplingFilter`.
#[derive(Debug, Clone, Copy)]
pub enum SamplingFilter {
    Lanczos3,
    Nearest,
    Linear,
    Cubic,
    Gaussian,
}

impl From<SamplingFilter> for image::imageops::FilterType {
    fn from(f: SamplingFilter) -> Self {
        match f {
            SamplingFilter::Lanczos3 => image::imageops::FilterType::Lanczos3,
            SamplingFilter::Nearest => image::imageops::FilterType::Nearest,
            SamplingFilter::Linear => image::imageops::FilterType::Triangle,
            SamplingFilter::Cubic => image::imageops::FilterType::CatmullRom,
            SamplingFilter::Gaussian => image::imageops::FilterType::Gaussian,
        }
    }
}

/// Resize `image` to exactly `(width, height)` using `filter`.
///
/// Mirrors `photon.resize(image, width, height, photon.SamplingFilter.Lanczos3)`.
pub fn resize(image: &PhotonImage, width: u32, height: u32, filter: SamplingFilter) -> PhotonImage {
    let resized = image.inner.resize_exact(width, height, filter.into());
    PhotonImage { inner: resized }
}

// ============================================================================
// Flip operations (in-place on DynamicImage)
// ============================================================================

/// Flip horizontally in-place. Mirrors `photon.fliph(image)`.
pub fn fliph(image: &mut PhotonImage) {
    image.inner = image.inner.fliph();
}

/// Flip vertically in-place. Mirrors `photon.flipv(image)`.
pub fn flipv(image: &mut PhotonImage) {
    image.inner = image.inner.flipv();
}

// ============================================================================
// Internal helpers
// ============================================================================

fn encode_image(img: &DynamicImage, format: ImageFormat) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), format)?;
    Ok(buf)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png() -> Vec<u8> {
        // 1x1 red pixel PNG
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::Png)
            .unwrap();
        buf
    }

    #[test]
    fn load_and_encode_png() {
        let bytes = tiny_png();
        let img = PhotonImage::new_from_byteslice(&bytes).unwrap();
        assert_eq!(img.get_width(), 1);
        assert_eq!(img.get_height(), 1);
        let out = img.get_bytes().unwrap();
        assert!(!out.is_empty());
    }

    #[test]
    fn resize_changes_dimensions() {
        let bytes = tiny_png();
        let img = PhotonImage::new_from_byteslice(&bytes).unwrap();
        // Create a 4x4 to give something to resize
        let bigger = resize(&img, 4, 4, SamplingFilter::Nearest);
        assert_eq!(bigger.get_width(), 4);
        assert_eq!(bigger.get_height(), 4);
    }

    #[test]
    fn fliph_does_not_panic() {
        let bytes = tiny_png();
        let mut img = PhotonImage::new_from_byteslice(&bytes).unwrap();
        fliph(&mut img);
    }
}
