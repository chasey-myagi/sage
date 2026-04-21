//! Async file watcher for user keybindings with write-finish debounce.
//!
//! Mirrors chokidar's `awaitWriteFinish` semantics: raw file-system events
//! are coalesced for `DEBOUNCE_MS` milliseconds before a change notification
//! fires.  This prevents partial-write reads when an editor saves atomically
//! via rename.
//!
//! CC reference: `keybindings/loadUserBindings.ts` — `initializeKeybindingWatcher`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::broadcast;

use super::keybindings::{KeybindingsConfig, load_keybindings_from_file};

// ============================================================================
// Constants
// ============================================================================

/// Time to wait for file writes to settle before reloading.
/// Mirrors chokidar `awaitWriteFinish.stabilityThreshold`.
const DEBOUNCE_MS: u64 = 500;

// ============================================================================
// Public API
// ============================================================================

/// A running keybinding file watcher.
///
/// Drop this value to stop the watcher.
pub struct KeybindingWatcher {
    // Keeps the underlying OS watcher alive.
    _watcher: RecommendedWatcher,
    /// Subscribe here to receive a notification after each settled file change.
    pub changes: broadcast::Receiver<KeybindingsConfig>,
}

/// Start watching `path` for changes and return a [`KeybindingWatcher`].
///
/// A background tokio task debounces raw events and reloads the file after
/// each settled write.  Subscribers receive the freshly-parsed config via the
/// broadcast channel on [`KeybindingWatcher::changes`].
///
/// Returns `Ok(None)` if the parent directory does not exist (nothing to
/// watch).
pub async fn start_keybinding_watcher(path: PathBuf) -> Result<Option<KeybindingWatcher>> {
    // Only watch if the parent directory exists.
    let watch_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    match tokio::fs::metadata(&watch_dir).await {
        Ok(m) if m.is_dir() => {}
        _ => return Ok(None),
    }

    let (change_tx, change_rx) = broadcast::channel::<KeybindingsConfig>(16);
    let (raw_tx, mut raw_rx) = tokio::sync::mpsc::unbounded_channel::<notify::Result<Event>>();

    let watcher_path = path.clone();
    let raw_tx_clone = raw_tx.clone();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = raw_tx_clone.send(res);
        },
        Config::default(),
    )
    .context("failed to create file watcher")?;

    // Watch the file if it exists; otherwise watch the directory so we catch
    // the file being created for the first time.
    let (watch_target, recursive) = if watcher_path.exists() {
        (watcher_path.clone(), RecursiveMode::NonRecursive)
    } else {
        (watch_dir, RecursiveMode::NonRecursive)
    };
    watcher
        .watch(&watch_target, recursive)
        .context("failed to start file watcher")?;

    // Debounce task: coalesce rapid events into a single reload.
    let debounce = Duration::from_millis(DEBOUNCE_MS);
    tokio::spawn(async move {
        let mut pending = false;
        let mut deadline = tokio::time::Instant::now() + debounce;

        loop {
            if pending {
                tokio::select! {
                    maybe = raw_rx.recv() => {
                        match maybe {
                            Some(Ok(event)) if is_relevant(&event, &watcher_path) => {
                                // Reset the debounce timer on each new event.
                                deadline = tokio::time::Instant::now() + debounce;
                            }
                            None => break, // sender dropped — watcher is gone
                            _ => {}
                        }
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        // Debounce window expired: reload and notify.
                        pending = false;
                        let config = load_keybindings_from_file(&watcher_path);
                        let _ = change_tx.send(config);
                    }
                }
            } else {
                match raw_rx.recv().await {
                    Some(Ok(event)) if is_relevant(&event, &watcher_path) => {
                        pending = true;
                        deadline = tokio::time::Instant::now() + debounce;
                    }
                    None => break,
                    _ => {}
                }
            }
        }
    });

    Ok(Some(KeybindingWatcher {
        _watcher: watcher,
        changes: change_rx,
    }))
}

// ============================================================================
// Helpers
// ============================================================================

fn is_relevant(event: &Event, watched_path: &Path) -> bool {
    // Accept Create, Modify, Remove events.
    let relevant_kind = matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    );
    if !relevant_kind {
        return false;
    }
    // If the event names specific paths, filter to our target file only.
    if event.paths.is_empty() {
        return true;
    }
    event.paths.iter().any(|p| p == watched_path)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_watcher_returns_none_for_missing_dir() {
        let result = start_keybinding_watcher(PathBuf::from("/nonexistent/dir/keybindings.json"))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_watcher_starts_for_existing_dir() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("keybindings.json");
        std::fs::write(&path, r#"{"app.interrupt": "escape"}"#).unwrap();

        let result = start_keybinding_watcher(path).await.unwrap();
        assert!(result.is_some(), "watcher should start when dir exists");
    }

    #[test]
    fn test_is_relevant_create_event() {
        use notify::{Event, EventKind, event::CreateKind};
        let path = PathBuf::from("/tmp/keybindings.json");
        let event = Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![path.clone()],
            attrs: Default::default(),
        };
        assert!(is_relevant(&event, &path));
    }

    #[test]
    fn test_is_relevant_filters_unrelated_path() {
        use notify::{Event, EventKind, event::DataChange, event::ModifyKind};
        let watched = PathBuf::from("/tmp/keybindings.json");
        let other = PathBuf::from("/tmp/other.json");
        let event = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![other],
            attrs: Default::default(),
        };
        assert!(!is_relevant(&event, &watched));
    }
}
