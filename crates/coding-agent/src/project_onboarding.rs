//! Project onboarding state tracking and step management.
//!
//! Translated from pi-mono `packages/coding-agent/src/projectOnboardingState.ts`.
//!
//! Key divergences from the TypeScript source:
//! - `shouldShowProjectOnboarding` is not memoized; computed on demand. The
//!   memoized version in TS caused stale UI state when CLAUDE.md was created
//!   mid-session (see .analysis/projectOnboardingState.ts.md, ISSUE HIGH).
//! - `get_steps` is called on demand for the same reason.
//! - Config persistence errors surface as `Result<_, ConfigError>` instead of
//!   being silently swallowed.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ============================================================================
// Error type
// ============================================================================

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ============================================================================
// Step types
// ============================================================================

/// Identifies each onboarding step. Replaces the string-keyed `Step.key` from TS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepKey {
    Workspace,
    ClaudeMd,
}

/// An onboarding step with completion and enablement state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub key: StepKey,
    pub text: String,
    pub is_complete: bool,
    pub is_completable: bool,
    pub is_enabled: bool,
}

// ============================================================================
// Project config
// ============================================================================

/// Per-project onboarding config persisted to `.sage/project.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConfig {
    #[serde(default)]
    pub has_completed_project_onboarding: bool,
    #[serde(default)]
    pub project_onboarding_seen_count: u32,
}

fn project_config_path(cwd: &Path) -> PathBuf {
    cwd.join(".sage").join("project.json")
}

pub fn get_project_config(cwd: &Path) -> Result<ProjectConfig, ConfigError> {
    let path = project_config_path(cwd);
    let Ok(data) = fs::read_to_string(&path) else {
        return Ok(ProjectConfig::default());
    };
    Ok(serde_json::from_str(&data)?)
}

pub fn save_project_config(
    cwd: &Path,
    updater: impl FnOnce(ProjectConfig) -> ProjectConfig,
) -> Result<(), ConfigError> {
    let path = project_config_path(cwd);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let updated = updater(get_project_config(cwd)?);
    let json = serde_json::to_string_pretty(&updated)?;
    fs::write(&path, json)?;
    Ok(())
}

// ============================================================================
// Step computation
// ============================================================================

fn is_dir_empty(path: &Path) -> bool {
    fs::read_dir(path)
        .map(|mut d| d.next().is_none())
        .unwrap_or(true)
}

/// Compute onboarding steps for the given working directory.
///
/// Called on demand — not memoized — so state always reflects the filesystem.
pub fn get_steps(cwd: &Path) -> Vec<Step> {
    let has_claude_md = cwd.join("CLAUDE.md").exists();
    let is_workspace_empty = is_dir_empty(cwd);

    vec![
        Step {
            key: StepKey::Workspace,
            text: "Ask Claude to create a new app or clone a repository".to_string(),
            // Always false: once the user creates/clones anything the workspace
            // becomes non-empty, disabling this step entirely. The step never
            // needs is_complete=true because is_project_onboarding_complete
            // only evaluates enabled steps. Matches TS source (getSteps ln 29).
            is_complete: false,
            is_completable: true,
            is_enabled: is_workspace_empty,
        },
        Step {
            key: StepKey::ClaudeMd,
            text: "Run /init to create a CLAUDE.md file with instructions for Claude".to_string(),
            is_complete: has_claude_md,
            is_completable: true,
            is_enabled: !is_workspace_empty,
        },
    ]
}

/// Returns true if all enabled completable steps are complete.
pub fn is_project_onboarding_complete(cwd: &Path) -> bool {
    get_steps(cwd)
        .into_iter()
        .filter(|s| s.is_completable && s.is_enabled)
        .all(|s| s.is_complete)
}

/// Mark onboarding complete in the project config if it just finished.
///
/// Short-circuits if `has_completed_project_onboarding` is already set,
/// to avoid unnecessary filesystem reads from `is_project_onboarding_complete`.
pub fn maybe_mark_project_onboarding_complete(cwd: &Path) -> Result<(), ConfigError> {
    if get_project_config(cwd)?.has_completed_project_onboarding {
        return Ok(());
    }
    if is_project_onboarding_complete(cwd) {
        save_project_config(cwd, |mut c| {
            c.has_completed_project_onboarding = true;
            c
        })?;
    }
    Ok(())
}

/// Whether to show the project onboarding UI.
///
/// Not memoized — evaluated fresh each call so the result stays correct
/// after CLAUDE.md is created or the seen count changes mid-session.
pub fn should_show_project_onboarding(cwd: &Path) -> Result<bool, ConfigError> {
    let config = get_project_config(cwd)?;
    if config.has_completed_project_onboarding
        || config.project_onboarding_seen_count >= 4
        || std::env::var("IS_DEMO").is_ok()
    {
        return Ok(false);
    }
    Ok(!is_project_onboarding_complete(cwd))
}

/// Increment the number of times onboarding has been shown this project.
pub fn increment_project_onboarding_seen_count(cwd: &Path) -> Result<(), ConfigError> {
    save_project_config(cwd, |mut c| {
        c.project_onboarding_seen_count += 1;
        c
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_cwd() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn empty_dir_has_workspace_step_enabled() {
        let dir = make_cwd();
        let steps = get_steps(dir.path());
        let ws = steps.iter().find(|s| s.key == StepKey::Workspace).unwrap();
        assert!(ws.is_enabled);
    }

    #[test]
    fn nonempty_dir_disables_workspace_step() {
        let dir = make_cwd();
        fs::write(dir.path().join("file.txt"), "hello").unwrap();
        let steps = get_steps(dir.path());
        let ws = steps.iter().find(|s| s.key == StepKey::Workspace).unwrap();
        assert!(!ws.is_enabled);
    }

    #[test]
    fn claude_md_completes_claudemd_step() {
        let dir = make_cwd();
        fs::write(dir.path().join("file.txt"), "x").unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "# guide").unwrap();
        let steps = get_steps(dir.path());
        let cm = steps.iter().find(|s| s.key == StepKey::ClaudeMd).unwrap();
        assert!(cm.is_complete);
        assert!(cm.is_enabled);
    }

    #[test]
    fn onboarding_complete_when_claude_md_exists_and_dir_nonempty() {
        let dir = make_cwd();
        fs::write(dir.path().join("file.txt"), "x").unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "# guide").unwrap();
        assert!(is_project_onboarding_complete(dir.path()));
    }

    #[test]
    fn onboarding_incomplete_when_no_claude_md() {
        let dir = make_cwd();
        fs::write(dir.path().join("file.txt"), "x").unwrap();
        assert!(!is_project_onboarding_complete(dir.path()));
    }

    #[test]
    fn default_config_returns_zeros() {
        let dir = make_cwd();
        let config = get_project_config(dir.path()).unwrap();
        assert!(!config.has_completed_project_onboarding);
        assert_eq!(config.project_onboarding_seen_count, 0);
    }

    #[test]
    fn increment_seen_count_persists() {
        let dir = make_cwd();
        increment_project_onboarding_seen_count(dir.path()).unwrap();
        increment_project_onboarding_seen_count(dir.path()).unwrap();
        let config = get_project_config(dir.path()).unwrap();
        assert_eq!(config.project_onboarding_seen_count, 2);
    }

    #[test]
    fn should_not_show_after_four_views() {
        let dir = make_cwd();
        for _ in 0..4 {
            increment_project_onboarding_seen_count(dir.path()).unwrap();
        }
        assert!(!should_show_project_onboarding(dir.path()).unwrap());
    }

    #[test]
    fn maybe_mark_complete_sets_flag() {
        let dir = make_cwd();
        fs::write(dir.path().join("file.txt"), "x").unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "# guide").unwrap();
        maybe_mark_project_onboarding_complete(dir.path()).unwrap();
        let config = get_project_config(dir.path()).unwrap();
        assert!(config.has_completed_project_onboarding);
    }

    #[test]
    fn maybe_mark_complete_noop_when_already_set() {
        let dir = make_cwd();
        save_project_config(dir.path(), |mut c| {
            c.has_completed_project_onboarding = true;
            c
        })
        .unwrap();
        maybe_mark_project_onboarding_complete(dir.path()).unwrap();
        let config = get_project_config(dir.path()).unwrap();
        assert!(config.has_completed_project_onboarding);
    }

    #[test]
    fn should_show_when_not_complete_and_unseen() {
        let dir = make_cwd();
        fs::write(dir.path().join("file.txt"), "x").unwrap();
        assert!(should_show_project_onboarding(dir.path()).unwrap());
    }

    #[test]
    fn should_not_show_when_completed() {
        let dir = make_cwd();
        fs::write(dir.path().join("file.txt"), "x").unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "# guide").unwrap();
        maybe_mark_project_onboarding_complete(dir.path()).unwrap();
        assert!(!should_show_project_onboarding(dir.path()).unwrap());
    }
}
