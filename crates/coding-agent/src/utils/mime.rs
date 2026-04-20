//! MIME type detection for image files.
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/mime.ts`.

use std::path::Path;

const FILE_TYPE_SNIFF_BYTES: usize = 4100;

const SUPPORTED_IMAGE_TYPES: &[(&[u8], &str)] = &[
    // JPEG: FF D8 FF
    (&[0xFF, 0xD8, 0xFF], "image/jpeg"),
    // PNG: 89 50 4E 47 0D 0A 1A 0A
    (&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A], "image/png"),
    // GIF87a / GIF89a
    (&[0x47, 0x49, 0x46, 0x38, 0x37, 0x61], "image/gif"),
    (&[0x47, 0x49, 0x46, 0x38, 0x39, 0x61], "image/gif"),
    // WebP: RIFF????WEBP
    // We detect the magic inline below due to the variable bytes.
];

fn detect_mime(buf: &[u8]) -> Option<&'static str> {
    // WebP: starts with RIFF and has WEBP at offset 8
    if buf.len() >= 12
        && &buf[0..4] == b"RIFF"
        && &buf[8..12] == b"WEBP"
    {
        return Some("image/webp");
    }

    for (magic, mime) in SUPPORTED_IMAGE_TYPES {
        if buf.len() >= magic.len() && &buf[..magic.len()] == *magic {
            return Some(mime);
        }
    }
    None
}

/// Detect if a file is a supported image (JPEG, PNG, GIF, WebP).
///
/// Returns the MIME type string, or `None` if the file is not a supported
/// image or cannot be read.
pub fn detect_supported_image_mime_type_from_file(file_path: &Path) -> anyhow::Result<Option<&'static str>> {
    let mut file = std::fs::File::open(file_path)?;
    let mut buf = vec![0u8; FILE_TYPE_SNIFF_BYTES];
    use std::io::Read;
    let bytes_read = file.read(&mut buf)?;
    if bytes_read == 0 {
        return Ok(None);
    }
    buf.truncate(bytes_read);
    Ok(detect_mime(&buf))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn detect_jpeg_magic() {
        let buf = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(detect_mime(&buf), Some("image/jpeg"));
    }

    #[test]
    fn detect_png_magic() {
        let buf = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00];
        assert_eq!(detect_mime(&buf), Some("image/png"));
    }

    #[test]
    fn detect_gif89a_magic() {
        let buf = b"GIF89a\x00\x00".to_vec();
        assert_eq!(detect_mime(&buf), Some("image/gif"));
    }

    #[test]
    fn detect_webp_magic() {
        let mut buf = b"RIFF\x00\x00\x00\x00WEBP".to_vec();
        buf.extend_from_slice(&[0x00; 20]);
        assert_eq!(detect_mime(&buf), Some("image/webp"));
    }

    #[test]
    fn unknown_type_returns_none() {
        let buf = b"hello world".to_vec();
        assert_eq!(detect_mime(&buf), None);
    }

    #[test]
    fn empty_buf_returns_none() {
        assert_eq!(detect_mime(&[]), None);
    }

    #[test]
    fn file_not_supported_image() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("file.txt");
        std::fs::write(&path, "hello").unwrap();
        let result = detect_supported_image_mime_type_from_file(&path).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn nonexistent_file_returns_error() {
        let result = detect_supported_image_mime_type_from_file(Path::new("/nonexistent/file.png"));
        assert!(result.is_err());
    }
}
