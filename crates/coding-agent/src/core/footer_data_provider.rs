//! Git branch and extension status data for the interactive-mode footer.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/footer-data-provider.ts`.
//!
//! Provides `FooterDataProvider` which tracks:
//! - The current git branch (resolved by reading `.git/HEAD`).
//! - Extension status strings set via `set_extension_status`.
//! - Available provider count (for display in the footer).
//!
//! The watch functionality from the TypeScript version (using `fs.watch`) is
//! intentionally omitted in this translation; branch refresh is synchronous
//! only.  A full async watcher can be added when the TUI layer is implemented.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ============================================================================
// Git resolution helpers
// ============================================================================

/// Walk upwards from `start` looking for a `.git` entry.
///
/// Handles both regular repos (`.git` is a directory) and worktrees (`.git`
/// is a file pointing at the real git dir).
fn find_git_head(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let git_path = dir.join(".git");
        if git_path.exists() {
            if git_path.is_file() {
                // Worktree: .git file contains "gitdir: <path>"
                let content = std::fs::read_to_string(&git_path).ok()?;
                if let Some(rest) = content.trim().strip_prefix("gitdir: ") {
                    let git_dir = dir.join(rest.trim());
                    let head = git_dir.join("HEAD");
                    if head.exists() {
                        return Some(head);
                    }
                }
            } else if git_path.is_dir() {
                let head = git_path.join("HEAD");
                if head.exists() {
                    return Some(head);
                }
            }
        }
        let parent = dir.parent()?;
        if parent == dir {
            return None;
        }
        dir = parent.to_path_buf();
    }
}

/// Read the current branch from a `HEAD` file.
///
/// - `ref: refs/heads/<name>` → returns `<name>`.
/// - Detached HEAD (bare SHA) → returns `"detached"`.
/// - Errors → returns `None`.
fn read_branch_from_head(head_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(head_path).ok()?;
    let content = content.trim();
    if let Some(branch) = content.strip_prefix("ref: refs/heads/") {
        Some(branch.to_string())
    } else {
        Some("detached".to_string())
    }
}

// ============================================================================
// FooterDataProvider
// ============================================================================

/// Data provider for the interactive-mode footer bar.
///
/// Mirrors pi-mono `FooterDataProvider`.
pub struct FooterDataProvider {
    /// Cached git branch (`None` = not yet resolved, `Some(None)` = not in repo).
    cached_branch: Option<Option<String>>,
    /// Path to the `.git/HEAD` file (if found).
    head_path: Option<PathBuf>,
    /// Extension status texts keyed by extension identifier.
    extension_statuses: HashMap<String, String>,
    /// Number of providers with available models.
    available_provider_count: usize,
}

impl FooterDataProvider {
    /// Create a new `FooterDataProvider` rooted at `cwd`.
    pub fn new(cwd: &Path) -> Self {
        let head_path = find_git_head(cwd);
        Self {
            cached_branch: None,
            head_path,
            extension_statuses: HashMap::new(),
            available_provider_count: 0,
        }
    }

    /// Create using `std::env::current_dir()` as the working directory.
    pub fn from_cwd() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::new(&cwd)
    }

    // ------------------------------------------------------------------ //
    // Public getters                                                       //
    // ------------------------------------------------------------------ //

    /// Current git branch, or `None` if not inside a git repository.
    ///
    /// `"detached"` is returned for detached-HEAD state.
    pub fn get_git_branch(&mut self) -> Option<&str> {
        if self.cached_branch.is_none() {
            let branch = self.head_path.as_deref().and_then(read_branch_from_head);
            self.cached_branch = Some(branch);
        }
        self.cached_branch.as_ref().unwrap().as_deref()
    }

    /// Refresh the cached branch from disk.
    pub fn refresh_git_branch(&mut self) {
        self.cached_branch = None;
    }

    /// Extension status strings set via `set_extension_status`.
    pub fn get_extension_statuses(&self) -> &HashMap<String, String> {
        &self.extension_statuses
    }

    /// Number of unique providers with available models.
    pub fn get_available_provider_count(&self) -> usize {
        self.available_provider_count
    }

    // ------------------------------------------------------------------ //
    // Internal setters (used by agent session / extension runner)          //
    // ------------------------------------------------------------------ //

    /// Set (or clear) the status text for `key`.
    pub fn set_extension_status(&mut self, key: &str, text: Option<&str>) {
        match text {
            Some(t) => {
                self.extension_statuses
                    .insert(key.to_string(), t.to_string());
            }
            None => {
                self.extension_statuses.remove(key);
            }
        }
    }

    /// Clear all extension statuses.
    pub fn clear_extension_statuses(&mut self) {
        self.extension_statuses.clear();
    }

    /// Update the available provider count.
    pub fn set_available_provider_count(&mut self, count: usize) {
        self.available_provider_count = count;
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn make_git_dir(head_content: &str) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();
        let mut head = std::fs::File::create(git_dir.join("HEAD")).unwrap();
        write!(head, "{}", head_content).unwrap();
        dir
    }

    #[test]
    fn get_git_branch_on_main() {
        let dir = make_git_dir("ref: refs/heads/main\n");
        let mut provider = FooterDataProvider::new(dir.path());
        assert_eq!(provider.get_git_branch(), Some("main"));
    }

    #[test]
    fn get_git_branch_detached() {
        let dir = make_git_dir("abc1234def5678\n");
        let mut provider = FooterDataProvider::new(dir.path());
        assert_eq!(provider.get_git_branch(), Some("detached"));
    }

    #[test]
    fn get_git_branch_not_in_repo() {
        let dir = tempfile::tempdir().unwrap();
        let mut provider = FooterDataProvider::new(dir.path());
        assert_eq!(provider.get_git_branch(), None);
    }

    #[test]
    fn branch_cached_after_first_call() {
        let dir = make_git_dir("ref: refs/heads/dev\n");
        let mut provider = FooterDataProvider::new(dir.path());
        // First call populates cache
        assert_eq!(provider.get_git_branch(), Some("dev"));
        // Overwrite HEAD on disk — cached value should be returned
        let head_path = dir.path().join(".git").join("HEAD");
        std::fs::write(&head_path, "ref: refs/heads/other\n").unwrap();
        assert_eq!(provider.get_git_branch(), Some("dev")); // still cached
    }

    #[test]
    fn refresh_clears_cache() {
        let dir = make_git_dir("ref: refs/heads/dev\n");
        let mut provider = FooterDataProvider::new(dir.path());
        assert_eq!(provider.get_git_branch(), Some("dev"));

        let head_path = dir.path().join(".git").join("HEAD");
        std::fs::write(&head_path, "ref: refs/heads/feature\n").unwrap();
        provider.refresh_git_branch();
        assert_eq!(provider.get_git_branch(), Some("feature"));
    }

    #[test]
    fn extension_status_set_and_get() {
        let dir = tempfile::tempdir().unwrap();
        let mut provider = FooterDataProvider::new(dir.path());
        provider.set_extension_status("my-ext", Some("running"));
        assert_eq!(
            provider
                .get_extension_statuses()
                .get("my-ext")
                .map(|s| s.as_str()),
            Some("running")
        );
    }

    #[test]
    fn extension_status_clear_with_none() {
        let dir = tempfile::tempdir().unwrap();
        let mut provider = FooterDataProvider::new(dir.path());
        provider.set_extension_status("my-ext", Some("running"));
        provider.set_extension_status("my-ext", None);
        assert!(!provider.get_extension_statuses().contains_key("my-ext"));
    }

    #[test]
    fn clear_extension_statuses() {
        let dir = tempfile::tempdir().unwrap();
        let mut provider = FooterDataProvider::new(dir.path());
        provider.set_extension_status("a", Some("1"));
        provider.set_extension_status("b", Some("2"));
        provider.clear_extension_statuses();
        assert!(provider.get_extension_statuses().is_empty());
    }

    #[test]
    fn available_provider_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut provider = FooterDataProvider::new(dir.path());
        assert_eq!(provider.get_available_provider_count(), 0);
        provider.set_available_provider_count(3);
        assert_eq!(provider.get_available_provider_count(), 3);
    }
}
