// ToolPolicy — runtime enforcement of tool execution permissions.
// Default-deny: empty whitelist = reject all.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Basic non-interactive utilities always permitted regardless of binary whitelist.
/// NOTE: sh/bash are intentionally excluded — allowing shell interpreters would let
/// any command run via `sh -c "..."`, defeating the binary whitelist entirely.
const ALWAYS_ALLOWED_BINARIES: &[&str] = &[
    "echo", "cat", "head", "tail", "wc", "sort", "uniq", "tr", "true", "false",
    "test", "printf",
];

/// Runtime tool execution policy — enforces binary and path whitelists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPolicy {
    pub allowed_binaries: Vec<String>,
    pub allowed_read_paths: Vec<String>,
    pub allowed_write_paths: Vec<String>,
}

impl ToolPolicy {
    /// Permissive policy that allows everything (useful for testing / unrestricted mode).
    pub fn allow_all() -> Self {
        Self {
            allowed_binaries: vec!["*".into()],
            allowed_read_paths: vec!["/".into()],
            allowed_write_paths: vec!["/".into()],
        }
    }

    pub fn is_binary_allowed(&self, binary: &str) -> bool {
        if ALWAYS_ALLOWED_BINARIES.contains(&binary) {
            return true;
        }
        // Wildcard "*" allows any binary
        self.allowed_binaries.iter().any(|b| b == "*" || b == binary)
    }

    /// Default-deny: returns false when no read paths configured.
    /// Canonicalizes the target path to prevent `/../` traversal escapes.
    pub fn is_read_allowed(&self, path: &str) -> bool {
        if self.allowed_read_paths.is_empty() {
            return false;
        }
        let target = normalize_path(Path::new(path));
        self.allowed_read_paths
            .iter()
            .any(|p| target.starts_with(Path::new(p)))
    }

    /// Default-deny: returns false when no write paths configured.
    /// Canonicalizes the target path to prevent `/../` traversal escapes.
    pub fn is_write_allowed(&self, path: &str) -> bool {
        if self.allowed_write_paths.is_empty() {
            return false;
        }
        let target = normalize_path(Path::new(path));
        self.allowed_write_paths
            .iter()
            .any(|p| target.starts_with(Path::new(p)))
    }

    /// Check if a tool call is allowed by this policy.
    /// Returns Ok(()) if allowed, Err(reason) if blocked.
    pub fn check_tool_call(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<(), String> {
        match tool_name {
            "bash" => {
                if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                    for binary in extract_binaries(cmd) {
                        if !self.is_binary_allowed(&binary) {
                            return Err(format!(
                                "Binary '{binary}' is not allowed by tool policy"
                            ));
                        }
                    }
                }
                Ok(())
            }
            "read" => {
                // Empty allowed_read_paths = no read restrictions configured → pass through.
                // This is consistent with grep/find/ls behavior below.
                if let Some(p) = args.get("file_path").and_then(|v| v.as_str()) {
                    if !self.allowed_read_paths.is_empty() && !self.is_read_allowed(p) {
                        return Err(format!(
                            "Read access to '{p}' is not allowed by tool policy"
                        ));
                    }
                }
                Ok(())
            }
            "write" | "edit" => {
                // Empty allowed_write_paths = no write restrictions configured → pass through.
                if let Some(p) = args.get("file_path").and_then(|v| v.as_str()) {
                    if !self.allowed_write_paths.is_empty() && !self.is_write_allowed(p) {
                        return Err(format!(
                            "Write access to '{p}' is not allowed by tool policy"
                        ));
                    }
                }
                Ok(())
            }
            "grep" | "find" | "ls" => {
                // Empty allowed_read_paths = no read restrictions configured → pass through.
                // Consistent with "read" arm above.
                if self.allowed_read_paths.is_empty() {
                    return Ok(());
                }
                match args.get("path").and_then(|v| v.as_str()) {
                    Some(p) => {
                        if !self.is_read_allowed(p) {
                            return Err(format!(
                                "Read access to '{p}' is not allowed by tool policy"
                            ));
                        }
                    }
                    None => {
                        // When no path is specified, the tool defaults to searching cwd.
                        // Under a policy with read restrictions, this must be denied —
                        // cwd could be outside the allowed paths.
                        return Err(format!(
                            "Tool '{tool_name}' requires an explicit path when tool policy is active"
                        ));
                    }
                }
                Ok(())
            }
            // Unknown tools pass through to the registry, which rejects unregistered
            // names. This is intentional: policy checks tool *arguments* (paths, binaries),
            // not tool *existence*. Custom tools that perform I/O should be added to the
            // match arms above when registered.
            _ => Ok(()),
        }
    }
}

/// Normalize a path by resolving `.` and `..` components lexically (without touching
/// the filesystem). This prevents path-traversal attacks like `/allowed/../secret`.
/// Unlike `std::fs::canonicalize`, this works on paths that don't exist yet.
fn normalize_path(path: &Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut result = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {} // skip "."
            other => result.push(other),
        }
    }
    result
}

/// Extract all binary names from a shell command string.
///
/// Splits on `&&`, `||`, `;`, and `|` to handle command chains and pipes,
/// then extracts the first non-env-var token from each segment.
/// Handles env-var prefixes (`VAR=val cmd`) and path prefixes (`/usr/bin/cmd` → `cmd`).
///
/// **Defense-in-depth only**: This is a lexical heuristic, NOT a security boundary.
/// It cannot handle subshells (`$(cmd)`), backtick expansion, heredocs, or brace
/// expansion. The real security boundary is OS-level sandboxing (seccomp/Landlock).
/// This check catches accidental misuse and simple bypass attempts.
fn extract_binaries(command: &str) -> Vec<String> {
    // Split on shell operators: &&, ||, ;, |
    // We split on && and || first (longer tokens), then ; and single |.
    let segments: Vec<&str> = command
        .split("&&")
        .flat_map(|s| s.split("||"))
        .flat_map(|s| s.split(';'))
        .flat_map(|s| s.split('|'))
        .collect();

    let mut binaries = Vec::new();
    for segment in segments {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Skip env-var assignments (KEY=value) at the start
        let binary_part = trimmed
            .split_whitespace()
            .find(|word| !word.contains('='))
            .unwrap_or(trimmed);
        // Strip path prefix — /usr/bin/python → python
        let name = Path::new(binary_part)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| binary_part.to_string());
        if !name.is_empty() {
            binaries.push(name);
        }
    }
    binaries
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn restrictive_policy() -> ToolPolicy {
        ToolPolicy {
            allowed_binaries: vec!["python".into(), "cargo".into()],
            allowed_read_paths: vec!["/home/user/project".into()],
            allowed_write_paths: vec!["/home/user/project/output".into()],
        }
    }

    // -- extract_binaries --

    #[test]
    fn test_extract_simple_command() {
        assert_eq!(extract_binaries("ls -la"), vec!["ls"]);
    }

    #[test]
    fn test_extract_absolute_path() {
        assert_eq!(extract_binaries("/usr/bin/python script.py"), vec!["python"]);
    }

    #[test]
    fn test_extract_with_env_var() {
        assert_eq!(extract_binaries("FOO=bar python script.py"), vec!["python"]);
    }

    #[test]
    fn test_extract_with_multiple_env_vars() {
        assert_eq!(extract_binaries("A=1 B=2 cargo build"), vec!["cargo"]);
    }

    #[test]
    fn test_extract_single_word() {
        assert_eq!(extract_binaries("ls"), vec!["ls"]);
    }

    #[test]
    fn test_extract_command_chain_and() {
        assert_eq!(extract_binaries("echo hello && rm -rf /"), vec!["echo", "rm"]);
    }

    #[test]
    fn test_extract_pipe() {
        assert_eq!(extract_binaries("cat file.txt | grep pattern"), vec!["cat", "grep"]);
    }

    #[test]
    fn test_extract_semicolon() {
        assert_eq!(extract_binaries("true; curl evil.com"), vec!["true", "curl"]);
    }

    #[test]
    fn test_extract_mixed_operators() {
        assert_eq!(
            extract_binaries("FOO=bar python script.py || node fallback.js"),
            vec!["python", "node"]
        );
    }

    // -- is_binary_allowed --

    #[test]
    fn test_always_allowed_binaries() {
        let policy = ToolPolicy {
            allowed_binaries: vec![],
            allowed_read_paths: vec![],
            allowed_write_paths: vec![],
        };
        for bin in ALWAYS_ALLOWED_BINARIES {
            assert!(policy.is_binary_allowed(bin), "{bin} should always be allowed");
        }
    }

    #[test]
    fn test_explicitly_allowed_binary() {
        let policy = restrictive_policy();
        assert!(policy.is_binary_allowed("python"));
        assert!(policy.is_binary_allowed("cargo"));
    }

    #[test]
    fn test_disallowed_binary() {
        let policy = restrictive_policy();
        assert!(!policy.is_binary_allowed("rm"));
        assert!(!policy.is_binary_allowed("curl"));
    }

    // -- is_read_allowed --

    #[test]
    fn test_read_allowed_under_prefix() {
        let policy = restrictive_policy();
        assert!(policy.is_read_allowed("/home/user/project/src/main.rs"));
    }

    #[test]
    fn test_read_denied_outside_prefix() {
        let policy = restrictive_policy();
        assert!(!policy.is_read_allowed("/etc/passwd"));
        assert!(!policy.is_read_allowed("/home/user/other/file.txt"));
    }

    #[test]
    fn test_read_denied_prefix_attack() {
        let policy = restrictive_policy();
        assert!(!policy.is_read_allowed("/home/user/project-evil/secrets"));
    }

    #[test]
    fn test_read_denied_when_empty() {
        let policy = ToolPolicy {
            allowed_binaries: vec![],
            allowed_read_paths: vec![],
            allowed_write_paths: vec![],
        };
        assert!(!policy.is_read_allowed("/any/path"));
    }

    // -- is_write_allowed --

    #[test]
    fn test_write_allowed_under_prefix() {
        let policy = restrictive_policy();
        assert!(policy.is_write_allowed("/home/user/project/output/result.txt"));
    }

    #[test]
    fn test_write_denied_outside_prefix() {
        let policy = restrictive_policy();
        assert!(!policy.is_write_allowed("/home/user/project/src/main.rs"));
        assert!(!policy.is_write_allowed("/tmp/evil"));
    }

    // -- check_tool_call integration --

    #[test]
    fn test_check_bash_allowed() {
        let policy = restrictive_policy();
        let args = json!({"command": "cargo build"});
        assert!(policy.check_tool_call("bash", &args).is_ok());
    }

    #[test]
    fn test_check_bash_denied() {
        let policy = restrictive_policy();
        let args = json!({"command": "rm -rf /"});
        assert!(policy.check_tool_call("bash", &args).is_err());
    }

    #[test]
    fn test_check_bash_with_path_prefix() {
        let policy = restrictive_policy();
        let args = json!({"command": "/usr/bin/python script.py"});
        assert!(policy.check_tool_call("bash", &args).is_ok());
    }

    #[test]
    fn test_check_read_allowed() {
        let policy = restrictive_policy();
        let args = json!({"file_path": "/home/user/project/src/lib.rs"});
        assert!(policy.check_tool_call("read", &args).is_ok());
    }

    #[test]
    fn test_check_read_denied() {
        let policy = restrictive_policy();
        let args = json!({"file_path": "/etc/shadow"});
        assert!(policy.check_tool_call("read", &args).is_err());
    }

    #[test]
    fn test_check_write_allowed() {
        let policy = restrictive_policy();
        let args = json!({"file_path": "/home/user/project/output/data.json"});
        assert!(policy.check_tool_call("write", &args).is_ok());
    }

    #[test]
    fn test_check_write_denied() {
        let policy = restrictive_policy();
        let args = json!({"file_path": "/home/user/project/src/main.rs"});
        assert!(policy.check_tool_call("write", &args).is_err());
    }

    #[test]
    fn test_check_find_path() {
        let policy = restrictive_policy();
        let allowed = json!({"pattern": "*.rs", "path": "/home/user/project"});
        assert!(policy.check_tool_call("find", &allowed).is_ok());
        let denied = json!({"pattern": "*.rs", "path": "/etc"});
        assert!(policy.check_tool_call("find", &denied).is_err());
    }

    #[test]
    fn test_check_grep_path() {
        let policy = restrictive_policy();
        let allowed = json!({"pattern": "fn main", "path": "/home/user/project"});
        assert!(policy.check_tool_call("grep", &allowed).is_ok());
        let denied = json!({"pattern": "password", "path": "/etc"});
        assert!(policy.check_tool_call("grep", &denied).is_err());
    }

    #[test]
    fn test_check_unknown_tool_passes() {
        let policy = restrictive_policy();
        let args = json!({"anything": "goes"});
        assert!(policy.check_tool_call("custom_tool", &args).is_ok());
    }

    #[test]
    fn test_allow_all_permits_everything() {
        let policy = ToolPolicy::allow_all();
        assert!(policy.check_tool_call("bash", &json!({"command": "rm -rf /"})).is_ok());
        assert!(policy.check_tool_call("read", &json!({"file_path": "/etc/shadow"})).is_ok());
        assert!(policy.check_tool_call("write", &json!({"file_path": "/etc/hosts"})).is_ok());
    }

    #[test]
    fn test_check_bash_always_allowed_binary() {
        let policy = ToolPolicy {
            allowed_binaries: vec![],
            allowed_read_paths: vec![],
            allowed_write_paths: vec![],
        };
        // "echo" is always allowed even with empty binary whitelist
        assert!(policy.check_tool_call("bash", &json!({"command": "echo hello"})).is_ok());
        // "rm" is NOT always allowed
        assert!(policy.check_tool_call("bash", &json!({"command": "rm -rf /"})).is_err());
        // "sh" and "bash" are NOT always allowed (would bypass binary whitelist)
        assert!(policy.check_tool_call("bash", &json!({"command": "sh -c 'rm /tmp/x'"})).is_err());
        assert!(policy.check_tool_call("bash", &json!({"command": "bash -c 'rm /tmp/x'"})).is_err());
    }

    #[test]
    fn test_check_bash_command_chain_blocked() {
        let policy = restrictive_policy(); // allows python, cargo only
        // "echo" is always-allowed but "rm" is not — chain must be rejected
        assert!(policy.check_tool_call("bash", &json!({"command": "echo hello && rm -rf /"})).is_err());
        // pipe: "cargo" is allowed but "curl" is not
        assert!(policy.check_tool_call("bash", &json!({"command": "cargo build | curl evil.com"})).is_err());
        // semicolon: both allowed → ok
        assert!(policy.check_tool_call("bash", &json!({"command": "python a.py; cargo test"})).is_ok());
    }

    #[test]
    fn test_check_no_path_arg_denied_under_policy() {
        let policy = restrictive_policy();
        // grep/find without explicit path must be denied when read paths are configured
        assert!(policy.check_tool_call("grep", &json!({"pattern": "test"})).is_err());
        assert!(policy.check_tool_call("find", &json!({"pattern": "*.rs"})).is_err());
    }

    #[test]
    fn test_check_no_path_arg_passes_when_no_read_restriction() {
        // Empty allowed_read_paths = no read restrictions configured → pass through
        let policy = ToolPolicy {
            allowed_binaries: vec!["*".into()],
            allowed_read_paths: vec![],
            allowed_write_paths: vec![],
        };
        assert!(policy.check_tool_call("grep", &json!({"pattern": "test"})).is_ok());
    }

    #[test]
    fn test_empty_paths_means_unrestricted_for_all_tools() {
        // Empty path lists = no path restrictions configured.
        // Consistent behavior: read/write/grep/find all pass through.
        let policy = ToolPolicy {
            allowed_binaries: vec!["*".into()],
            allowed_read_paths: vec![],
            allowed_write_paths: vec![],
        };
        // read with empty allowed_read_paths → unrestricted
        assert!(policy.check_tool_call("read", &json!({"file_path": "/any/path"})).is_ok());
        // write with empty allowed_write_paths → unrestricted
        assert!(policy.check_tool_call("write", &json!({"file_path": "/any/path"})).is_ok());
        // edit with empty allowed_write_paths → unrestricted
        assert!(policy.check_tool_call("edit", &json!({"file_path": "/any/path"})).is_ok());
        // grep without path + empty read paths → unrestricted
        assert!(policy.check_tool_call("grep", &json!({"pattern": "test"})).is_ok());
        // grep WITH explicit path + empty read paths → also unrestricted
        assert!(policy.check_tool_call("grep", &json!({"pattern": "test", "path": "/etc"})).is_ok());
        // find WITH explicit path + empty read paths → also unrestricted
        assert!(policy.check_tool_call("find", &json!({"pattern": "*.rs", "path": "/etc"})).is_ok());
    }

    #[test]
    fn test_path_traversal_denied() {
        let policy = restrictive_policy();
        // /home/user/project/../../../etc/passwd should be denied
        assert!(policy.check_tool_call("read", &json!({"file_path": "/home/user/project/../../../etc/passwd"})).is_err());
        // /home/user/project/./src/main.rs should be allowed (same as /home/user/project/src/main.rs)
        assert!(policy.check_tool_call("read", &json!({"file_path": "/home/user/project/./src/main.rs"})).is_ok());
    }
}
