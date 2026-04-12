use serde::{Deserialize, Serialize};
use std::path::Path;

/// Sandbox enforcement policy derived from AgentConfig.tools (YAML configuration layer).
///
/// This struct represents the *config representation* of tool policies, parsed from
/// agent YAML files. It is converted to `agent_runtime::tools::policy::ToolPolicy`
/// for actual runtime enforcement in the agent loop.
///
/// **Important**: This struct only provides primitive checks (`is_binary_allowed`,
/// `is_read_allowed`, `is_write_allowed`). For integrated tool-call enforcement
/// (bash binary extraction, grep/find path checks, etc.), use
/// `agent_runtime::tools::policy::ToolPolicy::check_tool_call` at runtime.
///
/// Mapped to:
/// - Guest-side: Landlock/seccomp rules (Phase 2+)
/// - Host-side: Volume mounts + binary whitelist pre-check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPolicy {
    pub allowed_binaries: Vec<String>,
    pub allowed_read_paths: Vec<String>,
    pub allowed_write_paths: Vec<String>,
}

/// Non-interactive utilities always permitted (aligned with agent-runtime).
/// sh/bash are excluded to prevent binary whitelist bypass via `sh -c "..."`.
const ALWAYS_ALLOWED_BINARIES: &[&str] = &[
    "echo", "cat", "head", "tail", "wc", "sort", "uniq", "tr", "true", "false",
    "test", "printf",
];

/// Normalize a path by resolving `.` and `..` components lexically (without touching
/// the filesystem). Prevents path-traversal attacks like `/allowed/../secret`.
fn normalize_path(path: &Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut result = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => { result.pop(); }
            Component::CurDir => {}
            other => result.push(other),
        }
    }
    result
}

impl ToolPolicy {
    /// Check if a binary is allowed by this policy.
    pub fn is_binary_allowed(&self, binary: &str) -> bool {
        if ALWAYS_ALLOWED_BINARIES.contains(&binary) {
            return true;
        }
        self.allowed_binaries.iter().any(|b| b == "*" || b == binary)
    }

    /// Check if a path is allowed for reading.
    /// Default-deny: returns false when no read paths are configured.
    /// Normalizes target path to prevent `/../` traversal escapes.
    pub fn is_read_allowed(&self, path: &str) -> bool {
        if self.allowed_read_paths.is_empty() {
            return false;
        }
        let target = normalize_path(Path::new(path));
        self.allowed_read_paths
            .iter()
            .any(|p| target.starts_with(Path::new(p)))
    }

    /// Check if a path is allowed for writing.
    /// Default-deny: returns false when no write paths are configured.
    /// Normalizes target path to prevent `/../` traversal escapes.
    pub fn is_write_allowed(&self, path: &str) -> bool {
        if self.allowed_write_paths.is_empty() {
            return false;
        }
        let target = normalize_path(Path::new(path));
        self.allowed_write_paths
            .iter()
            .any(|p| target.starts_with(Path::new(p)))
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
        for bin in ALWAYS_ALLOWED_BINARIES {
            assert!(policy.is_binary_allowed(bin), "{bin} should always be allowed");
        }
        // sh/bash must NOT be always-allowed (would bypass binary whitelist)
        assert!(!policy.is_binary_allowed("sh"));
        assert!(!policy.is_binary_allowed("bash"));
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
    fn read_denied_when_empty_default_deny() {
        let policy = policy_with_paths(&[], &[]);
        assert!(!policy.is_read_allowed("/any/path"));
        assert!(!policy.is_read_allowed("/etc/passwd"));
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

    #[test]
    fn read_denied_prefix_attack() {
        // "/home/user" must NOT match "/home/user-evil/secrets"
        let policy = policy_with_paths(&["/home/user"], &[]);
        assert!(!policy.is_read_allowed("/home/user-evil/secrets"));
        assert!(policy.is_read_allowed("/home/user/safe.txt"));
    }

    // --- Write path whitelist ---

    #[test]
    fn write_denied_when_empty_default_deny() {
        let policy = policy_with_paths(&[], &[]);
        assert!(!policy.is_write_allowed("/any/path"));
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
    fn write_denied_prefix_attack() {
        let policy = policy_with_paths(&[], &["/tmp"]);
        assert!(!policy.is_write_allowed("/tmp-evil/payload"));
        assert!(policy.is_write_allowed("/tmp/safe.txt"));
    }

    #[test]
    fn write_denied_traversal_attack() {
        let policy = policy_with_paths(&[], &["/tmp"]);
        assert!(!policy.is_write_allowed("/tmp/../etc/hosts"));
        assert!(policy.is_write_allowed("/tmp/safe.txt"));
    }

    #[test]
    fn read_denied_traversal_attack() {
        let policy = policy_with_paths(&["/home/user/docs"], &[]);
        assert!(!policy.is_read_allowed("/home/user/docs/../../../etc/passwd"));
        assert!(policy.is_read_allowed("/home/user/docs/./readme.md"));
    }

    #[test]
    fn multiple_read_paths() {
        let policy = policy_with_paths(&["/docs", "/config"], &[]);
        assert!(policy.is_read_allowed("/docs/readme.md"));
        assert!(policy.is_read_allowed("/config/app.yaml"));
        assert!(!policy.is_read_allowed("/secrets/key.pem"));
    }
}
