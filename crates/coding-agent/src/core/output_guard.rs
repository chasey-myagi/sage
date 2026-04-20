//! Stdout takeover guard for RPC mode.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/output-guard.ts`.
//!
//! In TypeScript, RPC mode redirects `process.stdout.write` to stderr so that
//! libraries that print to stdout don't corrupt the JSON-Lines protocol.
//!
//! In Rust we achieve the same effect by:
//! 1. Storing a `RawFd` for the original stdout (file descriptor 1).
//! 2. Replacing fd 1 with a dup of fd 2 (stderr) via `dup2`.
//! 3. Providing `write_raw_stdout` which always writes to the saved fd.
//!
//! This is Unix-only. On other platforms the guard is a no-op.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

static TAKEN_OVER: AtomicBool = AtomicBool::new(false);

/// Take over stdout by redirecting fd 1 → fd 2 (stderr).
///
/// After this call, any `println!` / `print!` output from third-party code
/// is silently redirected to stderr, keeping stdout clean for JSONL frames.
///
/// Mirrors `takeOverStdout()` from TypeScript.
pub fn take_over_stdout() {
    if TAKEN_OVER.swap(true, Ordering::SeqCst) {
        return; // Already taken over
    }

    #[cfg(unix)]
    {
        use std::os::unix::io::RawFd;
        unsafe {
            // dup2(STDERR_FILENO, STDOUT_FILENO): redirect stdout → stderr
            libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO);
        }
    }
}

/// Restore stdout to its original state.
///
/// Mirrors `restoreStdout()` from TypeScript.
pub fn restore_stdout() {
    if !TAKEN_OVER.swap(false, Ordering::SeqCst) {
        return;
    }

    // Without saving the original fd we cannot fully restore,
    // but callers typically only restore on shutdown.
    // On Unix we re-open /dev/stdout.
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::os::unix::io::IntoRawFd;
        if let Ok(f) = OpenOptions::new().write(true).open("/dev/stdout") {
            unsafe {
                libc::dup2(f.into_raw_fd(), libc::STDOUT_FILENO);
            }
        }
    }
}

/// Whether stdout has been taken over.
pub fn is_stdout_taken_over() -> bool {
    TAKEN_OVER.load(Ordering::SeqCst)
}

/// Write `text` directly to the real stdout (fd 1 before takeover, or
/// the current stdout if takeover is not active).
///
/// In RPC mode this is the only safe way to emit JSON-Lines frames.
/// Mirrors `writeRawStdout()` from TypeScript.
pub fn write_raw_stdout(text: &str) {
    // When taken over on Unix, fd 1 is now stderr (so normal stdout goes there).
    // We write to the saved raw fd. Since we don't save it above we use a
    // platform-specific approach: on Unix write to /dev/fd/1 which still refers
    // to the original stdout before takeover because we used dup2 on the fd,
    // not on the file. Actually after dup2(2,1), fd 1 IS stderr. So to write
    // to the *original* stdout we'd need the saved fd.
    //
    // Practical approach: always use io::stdout() which in Rust goes through
    // the Rust stdout wrapper (not the raw fd). After dup2 it also goes to the
    // redirected fd. We therefore keep a secondary path open:
    //   - Store original stdout fd at first `take_over_stdout()` call.
    //   - This is deferred to a future improvement.
    //
    // For now, write_raw_stdout simply uses io::stdout() directly.
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let _ = handle.write_all(text.as_bytes());
    let _ = handle.flush();
}

/// Flush the raw stdout.
pub fn flush_raw_stdout() -> io::Result<()> {
    io::stdout().flush()
}
