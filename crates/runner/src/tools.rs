use serde::{Deserialize, Serialize};

/// Sandbox enforcement policy derived from AgentConfig.tools.
///
/// This is mapped to:
/// - Guest-side: Landlock/seccomp rules (Phase 2+)
/// - Host-side: Volume mounts + binary whitelist pre-check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPolicy {
    pub allowed_binaries: Vec<String>,
    pub allowed_read_paths: Vec<String>,
    pub allowed_write_paths: Vec<String>,
}

impl ToolPolicy {
    /// Check if a binary is allowed by this policy.
    pub fn is_binary_allowed(&self, binary: &str) -> bool {
        // Always allow basic shell utilities
        let always_allowed = ["sh", "echo", "cat", "head", "tail", "wc", "sort", "uniq", "tr"];
        if always_allowed.contains(&binary) {
            return true;
        }
        self.allowed_binaries.iter().any(|b| b == binary)
    }

    /// Check if a path is allowed for reading.
    pub fn is_read_allowed(&self, path: &str) -> bool {
        if self.allowed_read_paths.is_empty() {
            return true; // no restriction
        }
        self.allowed_read_paths
            .iter()
            .any(|p| path.starts_with(p))
    }

    /// Check if a path is allowed for writing.
    pub fn is_write_allowed(&self, path: &str) -> bool {
        if self.allowed_write_paths.is_empty() {
            return true;
        }
        self.allowed_write_paths
            .iter()
            .any(|p| path.starts_with(p))
    }
}
