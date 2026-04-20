//! Session picker CLI command.
//!
//! Translated from pi-mono `packages/coding-agent/src/cli/session-picker.ts`.
//!
//! In TypeScript, this shows a TUI session selector. In Rust we provide an
//! interactive CLI fallback that lists sessions and prompts the user to choose.

use std::cmp::Reverse;
use std::path::PathBuf;

// ============================================================================
// Types
// ============================================================================

/// Basic information about a session file.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub path: PathBuf,
    pub name: Option<String>,
    pub last_modified: Option<std::time::SystemTime>,
}

/// A function type for loading sessions (with optional progress).
pub type SessionsLoader = Box<dyn Fn() -> Vec<SessionInfo>>;

// ============================================================================
// Session selector
// ============================================================================

/// Show an interactive session selector and return the selected session path,
/// or `None` if the user cancelled.
///
/// Mirrors `selectSession()` from TypeScript (without the TUI — uses stdin).
pub fn select_session(sessions_loader: &SessionsLoader) -> Option<PathBuf> {
    let sessions = sessions_loader();

    if sessions.is_empty() {
        eprintln!("No sessions found.");
        return None;
    }

    println!("Available sessions:");
    for (i, session) in sessions.iter().enumerate() {
        let name = session.name.as_deref().unwrap_or_else(|| {
            session
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
        });
        println!("  {}. {}", i + 1, name);
        println!("     {}", session.path.display());
    }

    print!("\nSelect session (1-{}) or 0 to cancel: ", sessions.len());

    use std::io::Write;
    std::io::stdout().flush().ok();

    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return None;
    }

    let choice: usize = input.trim().parse().unwrap_or(0);
    if choice == 0 || choice > sessions.len() {
        return None;
    }

    Some(sessions[choice - 1].path.clone())
}

/// Discover session files in a directory.
pub fn load_sessions_from_dir(dir: &std::path::Path) -> Vec<SessionInfo> {
    let mut sessions = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return sessions,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "jsonl") {
            let last_modified = entry.metadata().ok().and_then(|m| m.modified().ok());
            sessions.push(SessionInfo {
                path,
                name: None,
                last_modified,
            });
        }
    }

    // Sort by last modified (newest first)
    sessions.sort_by_key(|b| Reverse(b.last_modified));

    sessions
}
