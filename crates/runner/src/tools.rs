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

#[cfg(test)]
mod tests {
    use super::*;

    fn policy_with_binaries(binaries: &[&str]) -> ToolPolicy {
        ToolPolicy {
            allowed_binaries: binaries.iter().map(|s| s.to_string()).collect(),
            allowed_read_paths: Vec::new(),
            allowed_write_paths: Vec::new(),
        }
    }

    fn policy_with_paths(read: &[&str], write: &[&str]) -> ToolPolicy {
        ToolPolicy {
            allowed_binaries: Vec::new(),
            allowed_read_paths: read.iter().map(|s| s.to_string()).collect(),
            allowed_write_paths: write.iter().map(|s| s.to_string()).collect(),
        }
    }

    // --- Binary whitelist ---

    #[test]
    fn always_allowed_binaries() {
        let policy = policy_with_binaries(&[]);
        for bin in ["sh", "echo", "cat", "head", "tail", "wc", "sort", "uniq", "tr"] {
            assert!(policy.is_binary_allowed(bin), "{bin} should always be allowed");
        }
    }

    #[test]
    fn explicitly_allowed_binary() {
        let policy = policy_with_binaries(&["python", "cargo"]);
        assert!(policy.is_binary_allowed("python"));
        assert!(policy.is_binary_allowed("cargo"));
    }

    #[test]
    fn disallowed_binary() {
        let policy = policy_with_binaries(&["python"]);
        assert!(!policy.is_binary_allowed("rm"));
        assert!(!policy.is_binary_allowed("curl"));
        assert!(!policy.is_binary_allowed("python3")); // exact match only
    }

    // --- Read path whitelist ---

    #[test]
    fn read_allowed_no_restriction() {
        let policy = policy_with_paths(&[], &[]);
        assert!(policy.is_read_allowed("/any/path"));
        assert!(policy.is_read_allowed("/etc/passwd"));
    }

    #[test]
    fn read_allowed_matching_prefix() {
        let policy = policy_with_paths(&["/home/user/docs"], &[]);
        assert!(policy.is_read_allowed("/home/user/docs/file.txt"));
        assert!(policy.is_read_allowed("/home/user/docs/sub/nested.md"));
    }

    #[test]
    fn read_denied_non_matching() {
        let policy = policy_with_paths(&["/home/user/docs"], &[]);
        assert!(!policy.is_read_allowed("/tmp/secret"));
        assert!(!policy.is_read_allowed("/home/user/other"));
    }

    // --- Write path whitelist ---

    #[test]
    fn write_allowed_no_restriction() {
        let policy = policy_with_paths(&[], &[]);
        assert!(policy.is_write_allowed("/any/path"));
    }

    #[test]
    fn write_allowed_matching_prefix() {
        let policy = policy_with_paths(&[], &["/tmp", "/home/user/src"]);
        assert!(policy.is_write_allowed("/tmp/output.txt"));
        assert!(policy.is_write_allowed("/home/user/src/main.rs"));
    }

    #[test]
    fn write_denied_non_matching() {
        let policy = policy_with_paths(&[], &["/tmp"]);
        assert!(!policy.is_write_allowed("/etc/hosts"));
        assert!(!policy.is_write_allowed("/home/user/src/main.rs"));
    }

    #[test]
    fn multiple_read_paths() {
        let policy = policy_with_paths(&["/docs", "/config"], &[]);
        assert!(policy.is_read_allowed("/docs/readme.md"));
        assert!(policy.is_read_allowed("/config/app.yaml"));
        assert!(!policy.is_read_allowed("/secrets/key.pem"));
    }
}
