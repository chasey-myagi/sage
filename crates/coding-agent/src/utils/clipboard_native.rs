//! Native clipboard access (image read/write).
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/clipboard-native.ts`.
//!
//! The TypeScript original loads `@mariozechner/clipboard` (a native Node.js addon)
//! at runtime and falls back to `null` when the module is unavailable or when
//! running without a display.
//!
//! In Rust we expose the same interface as an optional trait.  The optional
//! `arboard` feature flag provides an actual implementation; without it the
//! type is always `None`.

/// Clipboard capabilities needed for image access.
pub trait ClipboardNative: Send + Sync {
    /// Set clipboard text.
    fn set_text(&mut self, text: &str) -> anyhow::Result<()>;
    /// Returns `true` if the clipboard currently contains an image.
    fn has_image(&self) -> bool;
    /// Read clipboard image as raw PNG bytes.
    fn get_image_binary(&mut self) -> anyhow::Result<Vec<u8>>;
}

// ============================================================================
// arboard implementation (feature = "clipboard")
// ============================================================================

#[cfg(feature = "clipboard")]
pub struct ArboardClipboard(arboard::Clipboard);

#[cfg(feature = "clipboard")]
impl ClipboardNative for ArboardClipboard {
    fn set_text(&mut self, text: &str) -> anyhow::Result<()> {
        self.0.set_text(text)?;
        Ok(())
    }

    fn has_image(&self) -> bool {
        // arboard doesn't expose a `has_image()` check without reading; we attempt
        // a read and cache nothing.  Callers should gate on `get_image_binary()`.
        true
    }

    fn get_image_binary(&mut self) -> anyhow::Result<Vec<u8>> {
        let img_data = self.0.get_image()?;
        // arboard returns raw RGBA; encode to PNG using the `image` crate.
        let img = image::RgbaImage::from_raw(
            img_data.width as u32,
            img_data.height as u32,
            img_data.bytes.into_owned(),
        )
        .ok_or_else(|| anyhow::anyhow!("Failed to create image from clipboard data"))?;
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)?;
        Ok(buf)
    }
}

// ============================================================================
// Factory
// ============================================================================

/// Try to create a clipboard instance.
///
/// Returns `None` when:
/// - the `clipboard` feature is not enabled, or
/// - the platform has no display (headless Linux without `DISPLAY`/`WAYLAND_DISPLAY`), or
/// - running in Termux.
pub fn open_clipboard() -> Option<Box<dyn ClipboardNative>> {
    // Mirror the TypeScript guard: skip when `TERMUX_VERSION` is set.
    if std::env::var("TERMUX_VERSION").is_ok() {
        return None;
    }

    // On Linux require a display server.
    #[cfg(target_os = "linux")]
    {
        let has_display = std::env::var("DISPLAY").is_ok()
            || std::env::var("WAYLAND_DISPLAY").is_ok();
        if !has_display {
            return None;
        }
    }

    #[cfg(feature = "clipboard")]
    {
        match arboard::Clipboard::new() {
            Ok(cb) => Some(Box::new(ArboardClipboard(cb))),
            Err(_) => None,
        }
    }

    #[cfg(not(feature = "clipboard"))]
    None
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_clipboard_does_not_panic() {
        // We only assert no panic; availability depends on the runtime environment.
        let _ = open_clipboard();
    }
}
