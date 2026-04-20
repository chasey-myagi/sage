//! Clipboard utilities.
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/clipboard.ts`.
//!
//! Strategy (same as TypeScript):
//! 1. Always emit OSC 52 — works over SSH/mosh, harmless locally.
//! 2. Try the `arboard` crate for native clipboard access.
//! 3. Fall back to platform-specific CLI tools (pbcopy, clip, wl-copy, xclip/xsel).

use std::process::{Command, Stdio};

/// Copy `text` to the system clipboard.
pub fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    // Always emit OSC 52 as a universal fallback (SSH-safe).
    emit_osc52(text);

    // Try arboard (cross-platform native clipboard).
    #[cfg(feature = "clipboard")]
    {
        if try_arboard(text) {
            return Ok(());
        }
    }

    // Platform-specific CLI tools (best-effort).
    try_platform_clipboard(text);

    Ok(())
}

fn emit_osc52(text: &str) {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    // Write directly to stdout without buffering.
    use std::io::Write;
    let _ = std::io::stdout().write_all(format!("\x1b]52;c;{encoded}\x07").as_bytes());
}

#[cfg(feature = "clipboard")]
fn try_arboard(text: &str) -> bool {
    match arboard::Clipboard::new() {
        Ok(mut board) => board.set_text(text).is_ok(),
        Err(_) => false,
    }
}

fn try_platform_clipboard(text: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = run_stdin_command("pbcopy", &[], text);
    }

    #[cfg(target_os = "windows")]
    {
        let _ = run_stdin_command("clip", &[], text);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Linux: try Termux, Wayland, X11 in order.
        if std::env::var("TERMUX_VERSION").is_ok() {
            if run_stdin_command("termux-clipboard-set", &[], text).is_ok() {
                return;
            }
        }

        let has_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
        let has_x11 = std::env::var("DISPLAY").is_ok();

        if has_wayland {
            if run_stdin_command("wl-copy", &[], text).is_ok() {
                return;
            }
        }

        if has_x11 {
            if run_stdin_command("xclip", &["-selection", "clipboard"], text).is_err() {
                let _ = run_stdin_command("xsel", &["--clipboard", "--input"], text);
            }
        }
    }
}

fn run_stdin_command(cmd: &str, args: &[&str], input: &str) -> anyhow::Result<()> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    if let Some(stdin) = child.stdin.take() {
        use std::io::Write;
        let mut stdin = stdin;
        stdin.write_all(input.as_bytes())?;
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("{} failed", cmd))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_osc52_does_not_panic() {
        // Just verify it doesn't panic; we can't assert stdout in unit tests.
        emit_osc52("hello world");
    }

    #[test]
    fn copy_to_clipboard_returns_ok() {
        // The function is best-effort; it always returns Ok regardless of
        // whether the native clipboard is available.
        let result = copy_to_clipboard("test");
        assert!(result.is_ok());
    }
}
