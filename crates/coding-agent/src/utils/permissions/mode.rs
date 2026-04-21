//! Permission mode types.
//!
//! Translated from pi-mono `utils/permissions/PermissionMode.ts`
//! and `types/permissions.ts`.

use serde::{Deserialize, Serialize};

/// The current permission mode controlling how tool calls are handled.
///
/// `transcript_classifier` feature gates the `Auto` variant.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    /// Normal interactive mode — prompts user for tool approval.
    #[default]
    Default,
    /// Plan mode — read-only exploration, no file edits allowed.
    Plan,
    /// Automatically accept file edits without prompting.
    AcceptEdits,
    /// Skip all permission checks (dangerous).
    BypassPermissions,
    /// Never ask for permission — deny instead of prompting.
    DontAsk,
    /// Auto mode with transcript classifier (ant-internal).
    #[cfg(feature = "transcript_classifier")]
    Auto,
}

impl PermissionMode {
    pub fn title(&self) -> &'static str {
        match self {
            PermissionMode::Default => "Default",
            PermissionMode::Plan => "Plan Mode",
            PermissionMode::AcceptEdits => "Accept edits",
            PermissionMode::BypassPermissions => "Bypass Permissions",
            PermissionMode::DontAsk => "Don't Ask",
            #[cfg(feature = "transcript_classifier")]
            PermissionMode::Auto => "Auto mode",
        }
    }

    pub fn short_title(&self) -> &'static str {
        match self {
            PermissionMode::Default => "Default",
            PermissionMode::Plan => "Plan",
            PermissionMode::AcceptEdits => "Accept",
            PermissionMode::BypassPermissions => "Bypass",
            PermissionMode::DontAsk => "DontAsk",
            #[cfg(feature = "transcript_classifier")]
            PermissionMode::Auto => "Auto",
        }
    }

    pub fn is_default(&self) -> bool {
        matches!(self, PermissionMode::Default)
    }

    /// Parse from a string, falling back to `Default` on unknown values.
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "plan" => PermissionMode::Plan,
            "acceptEdits" => PermissionMode::AcceptEdits,
            "bypassPermissions" => PermissionMode::BypassPermissions,
            "dontAsk" => PermissionMode::DontAsk,
            #[cfg(feature = "transcript_classifier")]
            "auto" => PermissionMode::Auto,
            _ => PermissionMode::Default,
        }
    }
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            PermissionMode::Default => "default",
            PermissionMode::Plan => "plan",
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::BypassPermissions => "bypassPermissions",
            PermissionMode::DontAsk => "dontAsk",
            #[cfg(feature = "transcript_classifier")]
            PermissionMode::Auto => "auto",
        };
        write!(f, "{s}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_display() {
        assert_eq!(PermissionMode::Default.to_string(), "default");
    }

    #[test]
    fn plan_mode_title() {
        assert_eq!(PermissionMode::Plan.title(), "Plan Mode");
    }

    #[test]
    fn from_str_lossy_known() {
        assert_eq!(PermissionMode::from_str_lossy("plan"), PermissionMode::Plan);
        assert_eq!(
            PermissionMode::from_str_lossy("bypassPermissions"),
            PermissionMode::BypassPermissions
        );
    }

    #[test]
    fn from_str_lossy_unknown_falls_back_to_default() {
        assert_eq!(
            PermissionMode::from_str_lossy("unknown"),
            PermissionMode::Default
        );
    }

    #[test]
    fn is_default_only_for_default() {
        assert!(PermissionMode::Default.is_default());
        assert!(!PermissionMode::Plan.is_default());
    }
}
