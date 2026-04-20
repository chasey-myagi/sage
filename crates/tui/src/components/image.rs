/// Image component — renders terminal images using Kitty or iTerm2 protocol.
/// Mirrors pi-mono `components/image.ts`.

use std::cell::RefCell;

use crate::terminal_image::{
    get_capabilities, get_image_dimensions, image_fallback, render_image, ImageDimensions,
    ImageRenderOptions,
};
use crate::tui::Component;

type FallbackColorFn = Box<dyn Fn(&str) -> String + Send + Sync>;

pub struct ImageTheme {
    pub fallback_color: FallbackColorFn,
}

impl ImageTheme {
    pub fn new<F>(fallback_color: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        Self { fallback_color: Box::new(fallback_color) }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ImageOptions {
    pub max_width_cells: Option<u32>,
    pub max_height_cells: Option<u32>,
    pub filename: Option<String>,
    /// Kitty image ID (for reuse / animations).
    pub image_id: Option<u32>,
}

pub struct Image {
    base64_data: String,
    mime_type: String,
    dimensions: ImageDimensions,
    theme: ImageTheme,
    options: ImageOptions,
    image_id: Option<u32>,

    cached_lines: RefCell<Option<Vec<String>>>,
    cached_width: RefCell<Option<u16>>,
}

impl Image {
    pub fn new(
        base64_data: impl Into<String>,
        mime_type: impl Into<String>,
        theme: ImageTheme,
        options: ImageOptions,
        dimensions: Option<ImageDimensions>,
    ) -> Self {
        let base64_data = base64_data.into();
        let mime_type = mime_type.into();
        let dimensions = dimensions
            .or_else(|| get_image_dimensions(&base64_data, &mime_type))
            .unwrap_or(ImageDimensions { width_px: 800, height_px: 600 });
        let image_id = options.image_id;

        Self {
            base64_data,
            mime_type,
            dimensions,
            theme,
            options,
            image_id,
            cached_lines: RefCell::new(None),
            cached_width: RefCell::new(None),
        }
    }

    /// Get the Kitty image ID used by this image (if any).
    pub fn get_image_id(&self) -> Option<u32> {
        self.image_id
    }
}

impl Component for Image {
    fn render(&self, width: u16) -> Vec<String> {
        {
            let cl = self.cached_lines.borrow();
            let cw = self.cached_width.borrow();
            if let (Some(cl), Some(cw)) = (cl.as_ref(), cw.as_ref()) {
                if *cw == width {
                    return cl.clone();
                }
            }
        }

        let max_width = {
            let w_minus_2 = (width as i32 - 2).max(1) as u32;
            w_minus_2.min(self.options.max_width_cells.unwrap_or(60))
        };

        let caps = get_capabilities();
        let lines = if caps.images.is_some() {
            let opts = ImageRenderOptions {
                max_width_cells: Some(max_width),
                max_height_cells: self.options.max_height_cells,
                image_id: self.image_id,
                preserve_aspect_ratio: None,
            };
            let result = render_image(&self.base64_data, self.dimensions, &opts);

            if let Some((sequence, rows, _id)) = result {
                let mut ls = Vec::with_capacity(rows as usize);
                for _ in 0..rows.saturating_sub(1) {
                    ls.push(String::new());
                }
                let move_up = if rows > 1 {
                    format!("\x1b[{}A", rows - 1)
                } else {
                    String::new()
                };
                ls.push(format!("{move_up}{sequence}"));
                ls
            } else {
                let fallback = image_fallback(
                    &self.mime_type,
                    Some(self.dimensions),
                    self.options.filename.as_deref(),
                );
                vec![(self.theme.fallback_color)(&fallback)]
            }
        } else {
            let fallback = image_fallback(
                &self.mime_type,
                Some(self.dimensions),
                self.options.filename.as_deref(),
            );
            vec![(self.theme.fallback_color)(&fallback)]
        };

        *self.cached_lines.borrow_mut() = Some(lines.clone());
        *self.cached_width.borrow_mut() = Some(width);

        lines
    }

    fn invalidate(&mut self) {
        *self.cached_lines.borrow_mut() = None;
        *self.cached_width.borrow_mut() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_fallback_renders() {
        let img = Image::new(
            "",
            "image/png",
            ImageTheme::new(|s: &str| s.to_string()),
            ImageOptions::default(),
            Some(ImageDimensions { width_px: 100, height_px: 50 }),
        );
        let lines = img.render(40);
        assert!(!lines.is_empty());
    }
}
