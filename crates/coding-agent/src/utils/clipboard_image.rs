//! Read images from the system clipboard.
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/clipboard-image.ts`.
//!
//! Supports:
//! - macOS / Windows / Linux (native clipboard via arboard, when the `clipboard` feature is enabled)
//! - Linux Wayland via `wl-paste`
//! - Linux X11 via `xclip`
//! - WSL via `wl-paste` + PowerShell fallback

use std::io::Read as _;
use std::process::{Command, Stdio};

use crate::utils::clipboard_native::open_clipboard;
use crate::utils::photon::PhotonImage;

// ============================================================================
// Types
// ============================================================================

/// An image read from the clipboard.
#[derive(Debug, Clone)]
pub struct ClipboardImage {
    /// Raw image bytes.
    pub bytes: Vec<u8>,
    /// MIME type, e.g. `"image/png"`.
    pub mime_type: String,
}

const SUPPORTED_MIME_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];

const DEFAULT_LIST_TIMEOUT_MS: u64 = 1000;
const DEFAULT_READ_TIMEOUT_MS: u64 = 3000;
const DEFAULT_POWERSHELL_TIMEOUT_MS: u64 = 5000;
const DEFAULT_MAX_BUFFER_BYTES: usize = 50 * 1024 * 1024;

// ============================================================================
// Platform helpers
// ============================================================================

/// `true` when running in a Wayland session.
pub fn is_wayland_session() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|v| v == "wayland")
            .unwrap_or(false)
}

/// `true` when running inside WSL.
pub fn is_wsl() -> bool {
    if std::env::var("WSL_DISTRO_NAME").is_ok() || std::env::var("WSLENV").is_ok() {
        return true;
    }
    std::fs::read_to_string("/proc/version")
        .map(|v| {
            let lower = v.to_lowercase();
            lower.contains("microsoft") || lower.contains("wsl")
        })
        .unwrap_or(false)
}

fn base_mime_type(mime_type: &str) -> String {
    mime_type
        .split(';')
        .next()
        .unwrap_or(mime_type)
        .trim()
        .to_lowercase()
}

fn extension_for_mime_type(mime_type: &str) -> Option<&'static str> {
    match base_mime_type(mime_type).as_str() {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

fn is_supported_image_mime_type(mime_type: &str) -> bool {
    let base = base_mime_type(mime_type);
    SUPPORTED_MIME_TYPES.contains(&base.as_str())
}

fn select_preferred_mime_type(types: &[String]) -> Option<String> {
    let normalized: Vec<(String, String)> = types
        .iter()
        .filter(|t| !t.trim().is_empty())
        .map(|t| (t.clone(), base_mime_type(t)))
        .collect();

    for preferred in SUPPORTED_MIME_TYPES {
        if let Some((raw, _)) = normalized
            .iter()
            .find(|(_, base)| base.as_str() == *preferred)
        {
            return Some(raw.clone());
        }
    }

    // Fall back to any image/*
    normalized
        .into_iter()
        .find(|(_, base)| base.starts_with("image/"))
        .map(|(raw, _)| raw)
}

// ============================================================================
// Command runner
// ============================================================================

struct RunResult {
    stdout: Vec<u8>,
    ok: bool,
}

fn run_command(command: &str, args: &[&str], timeout_ms: u64, max_buffer: usize) -> RunResult {
    let result = Command::new(command)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();

    let mut child = match result {
        Ok(c) => c,
        Err(_) => {
            return RunResult {
                stdout: vec![],
                ok: false,
            };
        }
    };

    // Read stdout with a size cap.
    let mut stdout = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out
            .by_ref()
            .take(max_buffer as u64)
            .read_to_end(&mut stdout);
    }

    // Wait with timeout via thread::sleep + try_wait — acceptable for a CLI tool.
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(timeout_ms);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return RunResult {
                    stdout,
                    ok: status.success(),
                };
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return RunResult {
                        stdout: vec![],
                        ok: false,
                    };
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => {
                return RunResult {
                    stdout: vec![],
                    ok: false,
                };
            }
        }
    }
}

// ============================================================================
// Platform-specific readers
// ============================================================================

fn read_via_wl_paste() -> Option<ClipboardImage> {
    let list = run_command(
        "wl-paste",
        &["--list-types"],
        DEFAULT_LIST_TIMEOUT_MS,
        DEFAULT_MAX_BUFFER_BYTES,
    );
    if !list.ok {
        return None;
    }

    let types: Vec<String> = String::from_utf8_lossy(&list.stdout)
        .split('\n')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();

    let selected = select_preferred_mime_type(&types)?;

    let data = run_command(
        "wl-paste",
        &["--type", &selected, "--no-newline"],
        DEFAULT_READ_TIMEOUT_MS,
        DEFAULT_MAX_BUFFER_BYTES,
    );
    if !data.ok || data.stdout.is_empty() {
        return None;
    }

    Some(ClipboardImage {
        bytes: data.stdout,
        mime_type: base_mime_type(&selected),
    })
}

fn read_via_xclip() -> Option<ClipboardImage> {
    let targets = run_command(
        "xclip",
        &["-selection", "clipboard", "-t", "TARGETS", "-o"],
        DEFAULT_LIST_TIMEOUT_MS,
        DEFAULT_MAX_BUFFER_BYTES,
    );

    let candidate_types: Vec<String> = if targets.ok {
        String::from_utf8_lossy(&targets.stdout)
            .split('\n')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        vec![]
    };

    let preferred = if !candidate_types.is_empty() {
        select_preferred_mime_type(&candidate_types)
    } else {
        None
    };

    let try_types: Vec<String> = if let Some(p) = preferred {
        let mut v = vec![p];
        v.extend(SUPPORTED_MIME_TYPES.iter().map(|s| s.to_string()));
        v
    } else {
        SUPPORTED_MIME_TYPES.iter().map(|s| s.to_string()).collect()
    };

    for mime_type in try_types {
        let data = run_command(
            "xclip",
            &["-selection", "clipboard", "-t", &mime_type, "-o"],
            DEFAULT_READ_TIMEOUT_MS,
            DEFAULT_MAX_BUFFER_BYTES,
        );
        if data.ok && !data.stdout.is_empty() {
            return Some(ClipboardImage {
                bytes: data.stdout,
                mime_type: base_mime_type(&mime_type),
            });
        }
    }
    None
}

fn read_via_powershell() -> Option<ClipboardImage> {
    let tmp_file = {
        let uuid = {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        };
        std::env::temp_dir().join(format!("sage-wsl-clip-{uuid}.png"))
    };

    let tmp_path_str = tmp_file.to_string_lossy().to_string();

    let win_path_result = run_command(
        "wslpath",
        &["-w", &tmp_path_str],
        DEFAULT_LIST_TIMEOUT_MS,
        DEFAULT_MAX_BUFFER_BYTES,
    );
    if !win_path_result.ok {
        return None;
    }
    let win_path = String::from_utf8_lossy(&win_path_result.stdout)
        .trim()
        .to_owned();
    if win_path.is_empty() {
        return None;
    }

    let ps_script = [
        "Add-Type -AssemblyName System.Windows.Forms",
        "Add-Type -AssemblyName System.Drawing",
        "$path = $env:SAGE_WSL_CLIPBOARD_IMAGE_PATH",
        "$img = [System.Windows.Forms.Clipboard]::GetImage()",
        "if ($img) { $img.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); Write-Output 'ok' } else { Write-Output 'empty' }",
    ]
    .join("; ");

    let result = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &ps_script])
        .env("SAGE_WSL_CLIPBOARD_IMAGE_PATH", &win_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();

    let mut child = match result {
        Ok(c) => c,
        Err(_) => return None,
    };

    let mut output = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut output);
    }
    // Wait with timeout
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(DEFAULT_POWERSHELL_TIMEOUT_MS);
    loop {
        match child.try_wait() {
            Ok(Some(s)) if s.success() => break,
            Ok(Some(_)) => return None,
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                return None;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(_) => return None,
        }
    }

    if output.trim() != "ok" {
        return None;
    }

    let bytes = std::fs::read(&tmp_file).ok()?;
    let _ = std::fs::remove_file(&tmp_file);
    if bytes.is_empty() {
        return None;
    }
    Some(ClipboardImage {
        bytes,
        mime_type: "image/png".to_owned(),
    })
}

fn read_via_native_clipboard() -> Option<ClipboardImage> {
    let mut cb = open_clipboard()?;
    if !cb.has_image() {
        return None;
    }
    let bytes = cb.get_image_binary().ok()?;
    if bytes.is_empty() {
        return None;
    }
    Some(ClipboardImage {
        bytes,
        mime_type: "image/png".to_owned(),
    })
}

// ============================================================================
// Conversion fallback
// ============================================================================

/// Convert unsupported image formats (e.g. BMP from WSLg) to PNG.
fn convert_to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let image = PhotonImage::new_from_byteslice(bytes).ok()?;
    image.get_bytes().ok()
}

// ============================================================================
// Public API
// ============================================================================

/// Read an image from the system clipboard.
///
/// Returns `None` when:
/// - no image is on the clipboard
/// - running in Termux (no clipboard support)
/// - the image format cannot be converted to a supported type
///
/// Mirrors `readClipboardImage()` from TypeScript.
pub fn read_clipboard_image() -> Option<ClipboardImage> {
    // Skip Termux — it has no clipboard image API.
    if std::env::var("TERMUX_VERSION").is_ok() {
        return None;
    }

    // Assigned conditionally below depending on target_os.
    #[allow(unused_assignments)]
    let mut image: Option<ClipboardImage> = None;

    #[cfg(target_os = "linux")]
    {
        let wsl = is_wsl();
        let wayland = is_wayland_session();

        if wayland || wsl {
            image = read_via_wl_paste().or_else(read_via_xclip);
        }

        if image.is_none() && wsl {
            image = read_via_powershell();
        }

        if image.is_none() && !wayland {
            image = read_via_native_clipboard();
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        image = read_via_native_clipboard();
    }

    let img = image?;

    // Convert unsupported formats (e.g. BMP) to PNG.
    if !is_supported_image_mime_type(&img.mime_type) {
        let png_bytes = convert_to_png(&img.bytes)?;
        return Some(ClipboardImage {
            bytes: png_bytes,
            mime_type: "image/png".to_owned(),
        });
    }

    Some(img)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_mime_strips_params() {
        assert_eq!(base_mime_type("image/jpeg; charset=utf-8"), "image/jpeg");
        assert_eq!(base_mime_type("image/PNG"), "image/png");
    }

    #[test]
    fn supported_mime_check() {
        assert!(is_supported_image_mime_type("image/png"));
        assert!(is_supported_image_mime_type("image/JPEG"));
        assert!(!is_supported_image_mime_type("image/bmp"));
        assert!(!is_supported_image_mime_type("text/plain"));
    }

    #[test]
    fn select_preferred_prefers_png() {
        let types = vec![
            "image/bmp".to_owned(),
            "image/jpeg".to_owned(),
            "image/png".to_owned(),
        ];
        assert_eq!(select_preferred_mime_type(&types).unwrap(), "image/png");
    }

    #[test]
    fn select_preferred_falls_back_to_any_image() {
        let types = vec!["image/tiff".to_owned(), "text/html".to_owned()];
        assert_eq!(select_preferred_mime_type(&types).unwrap(), "image/tiff");
    }

    #[test]
    fn select_preferred_none_when_no_image() {
        let types = vec!["text/plain".to_owned()];
        assert!(select_preferred_mime_type(&types).is_none());
    }

    #[test]
    fn is_wayland_session_does_not_panic() {
        let _ = is_wayland_session();
    }
}
