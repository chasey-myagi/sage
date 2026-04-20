//! EXIF orientation detection and application.
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/exif-orientation.ts`.
//!
//! Reads the EXIF `Orientation` tag from JPEG and WebP images, then applies the
//! corresponding transform (flip/rotate) to a `PhotonImage`.

use crate::utils::photon::{PhotonImage, fliph, flipv};

// ============================================================================
// TIFF / EXIF parsing helpers
// ============================================================================

fn read_orientation_from_tiff(bytes: &[u8], tiff_start: usize) -> u16 {
    if tiff_start + 8 > bytes.len() {
        return 1;
    }

    let le = bytes[tiff_start] == 0x49 && bytes[tiff_start + 1] == 0x49;

    let read16 = |pos: usize| -> u16 {
        if le {
            (bytes[pos] as u16) | ((bytes[pos + 1] as u16) << 8)
        } else {
            ((bytes[pos] as u16) << 8) | (bytes[pos + 1] as u16)
        }
    };

    let read32 = |pos: usize| -> u32 {
        if le {
            (bytes[pos] as u32)
                | ((bytes[pos + 1] as u32) << 8)
                | ((bytes[pos + 2] as u32) << 16)
                | ((bytes[pos + 3] as u32) << 24)
        } else {
            ((bytes[pos] as u32) << 24)
                | ((bytes[pos + 1] as u32) << 16)
                | ((bytes[pos + 2] as u32) << 8)
                | (bytes[pos + 3] as u32)
        }
    };

    let ifd_offset = read32(tiff_start + 4) as usize;
    let ifd_start = tiff_start + ifd_offset;
    if ifd_start + 2 > bytes.len() {
        return 1;
    }

    let entry_count = read16(ifd_start) as usize;
    for i in 0..entry_count {
        let entry_pos = ifd_start + 2 + i * 12;
        if entry_pos + 12 > bytes.len() {
            return 1;
        }
        if read16(entry_pos) == 0x0112 {
            let value = read16(entry_pos + 8);
            return if (1..=8).contains(&value) { value } else { 1 };
        }
    }

    1
}

fn has_exif_header(bytes: &[u8], offset: usize) -> bool {
    bytes.len() >= offset + 6
        && bytes[offset] == 0x45     // E
        && bytes[offset + 1] == 0x78 // x
        && bytes[offset + 2] == 0x69 // i
        && bytes[offset + 3] == 0x66 // f
        && bytes[offset + 4] == 0x00
        && bytes[offset + 5] == 0x00
}

fn find_jpeg_tiff_offset(bytes: &[u8]) -> Option<usize> {
    let mut offset = 2usize;
    while offset + 1 < bytes.len() {
        if bytes[offset] != 0xff {
            return None;
        }
        let marker = bytes[offset + 1];
        if marker == 0xff {
            offset += 1;
            continue;
        }
        if marker == 0xe1 {
            if offset + 4 >= bytes.len() {
                return None;
            }
            let segment_start = offset + 4;
            if segment_start + 6 > bytes.len() {
                return None;
            }
            if !has_exif_header(bytes, segment_start) {
                return None;
            }
            return Some(segment_start + 6);
        }
        if offset + 4 > bytes.len() {
            return None;
        }
        let length = (bytes[offset + 2] as usize) << 8 | bytes[offset + 3] as usize;
        offset += 2 + length;
    }
    None
}

fn find_webp_tiff_offset(bytes: &[u8]) -> Option<usize> {
    let mut offset = 12usize;
    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size = (bytes[offset + 4] as usize)
            | ((bytes[offset + 5] as usize) << 8)
            | ((bytes[offset + 6] as usize) << 16)
            | ((bytes[offset + 7] as usize) << 24);
        let data_start = offset + 8;

        if chunk_id == b"EXIF" {
            if data_start + chunk_size > bytes.len() {
                return None;
            }
            let tiff_start = if chunk_size >= 6 && has_exif_header(bytes, data_start) {
                data_start + 6
            } else {
                data_start
            };
            return Some(tiff_start);
        }

        offset = data_start + chunk_size + (chunk_size % 2);
    }
    None
}

fn get_exif_orientation(bytes: &[u8]) -> u16 {
    let tiff_offset: Option<usize>;

    // JPEG: starts with FF D8
    if bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] == 0xd8 {
        tiff_offset = find_jpeg_tiff_offset(bytes);
    }
    // WebP: starts with RIFF....WEBP
    else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        tiff_offset = find_webp_tiff_offset(bytes);
    } else {
        return 1;
    }

    match tiff_offset {
        Some(offset) => read_orientation_from_tiff(bytes, offset),
        None => 1,
    }
}

// ============================================================================
// Rotation helpers
// ============================================================================

/// Rotate 90° using a pixel-copying strategy mirroring the TypeScript `rotate90`.
///
/// `dst_index(x, y, w, h)` maps source pixel (x, y) → destination flat index.
/// Output dimensions are swapped (h × w becomes w × h after the rotate).
fn rotate90<F>(image: &PhotonImage, dst_index: F) -> PhotonImage
where
    F: Fn(u32, u32, u32, u32) -> u32,
{
    let w = image.get_width();
    let h = image.get_height();
    let src = image.get_raw_pixels();
    let mut dst = vec![0u8; src.len()];

    for y in 0..h {
        for x in 0..w {
            let src_idx = (y * w + x) as usize * 4;
            let dst_idx = dst_index(x, y, w, h) as usize * 4;
            dst[dst_idx..dst_idx + 4].copy_from_slice(&src[src_idx..src_idx + 4]);
        }
    }

    // After rotation the dimensions swap: new width = h, new height = w
    PhotonImage::from_raw_rgba(dst, h, w)
}

// ============================================================================
// Public API
// ============================================================================

/// Apply the EXIF orientation transform to `image`.
///
/// Translates `applyExifOrientation()` from TypeScript.
/// Returns the (possibly new) `PhotonImage` with correct orientation.
/// When the orientation is already `1` (normal) the same image is returned unmodified.
pub fn apply_exif_orientation(mut image: PhotonImage, original_bytes: &[u8]) -> PhotonImage {
    let orientation = get_exif_orientation(original_bytes);
    if orientation == 1 {
        return image;
    }

    match orientation {
        2 => {
            fliph(&mut image);
            image
        }
        3 => {
            fliph(&mut image);
            flipv(&mut image);
            image
        }
        4 => {
            flipv(&mut image);
            image
        }
        5 => {
            let h = image.get_height();
            let mut rotated = rotate90(&image, |x, y, _w, _h| x * h + (h - 1 - y));
            fliph(&mut rotated);
            rotated
        }
        6 => rotate90(&image, |x, y, _w, h| x * h + (h - 1 - y)),
        7 => {
            let w = image.get_width();
            let h = image.get_height();
            let mut rotated = rotate90(&image, |x, y, _w, _h| (w - 1 - x) * h + y);
            fliph(&mut rotated);
            rotated
        }
        8 => {
            let w = image.get_width();
            let h = image.get_height();
            rotate90(&image, |x, y, _w, _h| (w - 1 - x) * h + y)
        }
        _ => image,
    }
}

// ============================================================================
// Trait extension for slice length
// ============================================================================

trait SliceLen {
    fn length(&self) -> usize;
}

impl SliceLen for [u8] {
    fn length(&self) -> usize {
        self.len()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::photon::PhotonImage;
    use image::{DynamicImage, RgbaImage};

    fn make_2x1_image() -> PhotonImage {
        let img = RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
        PhotonImage::from_raw_rgba(img.into_raw(), 2, 1)
    }

    #[test]
    fn no_exif_returns_orientation_1() {
        // Random bytes with no JPEG or WebP magic
        let bytes = vec![0u8; 100];
        assert_eq!(get_exif_orientation(&bytes), 1);
    }

    #[test]
    fn apply_orientation_1_returns_unmodified() {
        let bytes = vec![0u8; 10]; // no JPEG magic → orientation 1
        let img = make_2x1_image();
        let width = img.get_width();
        let height = img.get_height();
        let out = apply_exif_orientation(img, &bytes);
        assert_eq!(out.get_width(), width);
        assert_eq!(out.get_height(), height);
    }
}
