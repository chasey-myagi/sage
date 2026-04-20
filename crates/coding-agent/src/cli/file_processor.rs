//! Process `@file` CLI arguments into text content and image attachments.
//!
//! Translated from pi-mono `packages/coding-agent/src/cli/file-processor.ts`.

use std::path::Path;

use serde::{Deserialize, Serialize};

// ============================================================================
// Types
// ============================================================================

/// A processed image attachment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageContent {
    #[serde(rename = "type")]
    pub content_type: String, // "image"
    pub mime_type: String,
    /// Base64-encoded image data.
    pub data: String,
}

/// Result of processing `@file` arguments.
#[derive(Debug, Default)]
pub struct ProcessedFiles {
    pub text: String,
    pub images: Vec<ImageContent>,
}

/// Options for file processing.
#[derive(Debug, Default)]
pub struct ProcessFileOptions {
    /// Whether to auto-resize images. Default: `true`.
    pub auto_resize_images: Option<bool>,
}

// ============================================================================
// MIME type detection (simplified)
// ============================================================================

fn detect_image_mime_type(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

// ============================================================================
// Main processor
// ============================================================================

/// Process `@file` arguments into text content and image attachments.
///
/// Mirrors `processFileArguments()` from TypeScript.
pub fn process_file_arguments(
    file_args: &[String],
    options: Option<&ProcessFileOptions>,
) -> Result<ProcessedFiles, String> {
    let mut result = ProcessedFiles::default();

    for file_arg in file_args {
        let path = expand_path(file_arg);

        if !path.exists() {
            return Err(format!("File not found: {}", path.display()));
        }

        let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if metadata.len() == 0 {
            // Skip empty files
            continue;
        }

        if let Some(mime) = detect_image_mime_type(&path) {
            // Image file
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            let b64 = base64_encode(&bytes);

            result.text.push_str(&format!(
                "<file name=\"{}\">[Image: {}]</file>\n",
                path.display(),
                mime
            ));
            result.images.push(ImageContent {
                content_type: "image".to_string(),
                mime_type: mime.to_string(),
                data: b64,
            });
        } else {
            // Text file
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Could not read file {}: {}", path.display(), e))?;
            result.text.push_str(&format!(
                "<file name=\"{}\">\n{}\n</file>\n",
                path.display(),
                content
            ));
        }
    }

    Ok(result)
}

fn expand_path(p: &str) -> std::path::PathBuf {
    let trimmed = p.trim();
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    std::path::PathBuf::from(trimmed)
}

fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 {
            chunk[1] as usize
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            chunk[2] as usize
        } else {
            0
        };

        out.push(CHARS[(b0 >> 2)] as char);
        out.push(CHARS[((b0 & 3) << 4) | (b1 >> 4)] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((b1 & 0xf) << 2) | (b2 >> 6)] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[b2 & 0x3f] as char);
        } else {
            out.push('=');
        }
    }
    out
}
