use serde::{Deserialize, Serialize};
use std::path::Path;

use sage_runtime::tools::policy::ToolPolicy;

/// Expand a leading `~` in a path string to the user's home directory.
/// Returns the original string unchanged if it doesn't start with `~` or
/// if the home directory cannot be determined.
fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return Path::new(&home).join(rest).to_string_lossy().to_string();
        }
    } else if path == "~" {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return home.to_string_lossy().to_string();
        }
    }
    path.to_string()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentConfig {
    pub name: String,
    pub description: String,
    pub llm: LlmConfig,
    pub system_prompt: String,
    pub tools: ToolsConfig,
    pub constraints: Constraints,
    #[serde(default)]
    pub sandbox: Option<SandboxConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LlmConfig {
    pub provider: String,
    pub model: String,
    pub max_tokens: u32,
    /// Override base URL for the LLM API endpoint.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Override environment variable name for the API key.
    #[serde(default)]
    pub api_key_env: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SandboxConfig {
    #[serde(default = "default_cpus")]
    pub cpus: u32,
    #[serde(default = "default_memory_mib")]
    pub memory_mib: u32,
    #[serde(default)]
    pub network: bool,
}

fn default_cpus() -> u32 {
    1
}

fn default_memory_mib() -> u32 {
    512
}

/// Predefined toolset presets for common use cases.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Toolset {
    /// All 7 tools, bash allowed_binaries = ["*"].
    Coding,
    /// bash + read + grep + find + ls (no write/edit). Common ops binaries.
    Ops,
    /// bash + read. HTTP-focused binaries (curl, wget, jq, python).
    Web,
    /// read + grep + find + ls only. No bash, no write, no edit.
    Minimal,
    /// Same as Minimal — read-only access.
    Readonly,
}

/// Preset allowed binaries for ops toolset.
const OPS_BINARIES: &[&str] = &[
    "curl", "wget", "docker", "kubectl", "ssh", "scp", "systemctl", "journalctl",
    "ps", "top", "htop", "df", "du", "free", "netstat", "ss", "ping", "dig",
    "nslookup", "ip", "iptables", "tar", "gzip", "zip", "unzip", "jq", "yq",
    "awk", "sed",
];

/// Preset allowed binaries for web toolset.
const WEB_BINARIES: &[&str] = &["curl", "wget", "jq", "python", "python3", "node"];

fn strs_to_strings(strs: &[&str]) -> Vec<String> {
    strs.iter().map(|s| s.to_string()).collect()
}

/// An unrestricted PathToolConfig (empty allowed_paths means the tool is
/// enabled but path restriction is delegated to the ToolPolicy layer —
/// an empty list in the policy means default-deny at enforcement time).
fn unrestricted_path_tool() -> PathToolConfig {
    PathToolConfig {
        allowed_paths: vec![],
    }
}

impl Toolset {
    /// Expand a preset into a concrete ToolsConfig with default settings.
    pub fn expand(&self) -> ToolsConfig {
        match self {
            Toolset::Coding => ToolsConfig {
                toolset: None,
                bash: Some(BashToolConfig {
                    allowed_binaries: vec!["*".to_string()],
                }),
                read: Some(unrestricted_path_tool()),
                write: Some(unrestricted_path_tool()),
                edit: Some(unrestricted_path_tool()),
                grep: Some(EmptyToolConfig {}),
                find: Some(EmptyToolConfig {}),
                ls: Some(EmptyToolConfig {}),
            },
            Toolset::Ops => ToolsConfig {
                toolset: None,
                bash: Some(BashToolConfig {
                    allowed_binaries: strs_to_strings(OPS_BINARIES),
                }),
                read: Some(unrestricted_path_tool()),
                write: None,
                edit: None,
                grep: Some(EmptyToolConfig {}),
                find: Some(EmptyToolConfig {}),
                ls: Some(EmptyToolConfig {}),
            },
            Toolset::Web => ToolsConfig {
                toolset: None,
                bash: Some(BashToolConfig {
                    allowed_binaries: strs_to_strings(WEB_BINARIES),
                }),
                read: Some(unrestricted_path_tool()),
                write: None,
                edit: None,
                grep: None,
                find: None,
                ls: None,
            },
            Toolset::Minimal | Toolset::Readonly => ToolsConfig {
                toolset: None,
                bash: None,
                read: Some(unrestricted_path_tool()),
                write: None,
                edit: None,
                grep: Some(EmptyToolConfig {}),
                find: Some(EmptyToolConfig {}),
                ls: Some(EmptyToolConfig {}),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Optional toolset preset — provides defaults for tool fields below.
    /// Explicit tool fields override the preset.
    #[serde(default)]
    pub toolset: Option<Toolset>,
    #[serde(default)]
    pub bash: Option<BashToolConfig>,
    #[serde(default)]
    pub read: Option<PathToolConfig>,
    #[serde(default)]
    pub write: Option<PathToolConfig>,
    #[serde(default)]
    pub edit: Option<PathToolConfig>,
    #[serde(default)]
    pub grep: Option<EmptyToolConfig>,
    #[serde(default)]
    pub find: Option<EmptyToolConfig>,
    #[serde(default)]
    pub ls: Option<EmptyToolConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BashToolConfig {
    pub allowed_binaries: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathToolConfig {
    pub allowed_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmptyToolConfig {}

#[derive(Debug, Serialize, Deserialize)]
pub struct Constraints {
    pub max_turns: u32,
    pub timeout_secs: u32,
}

impl ToolsConfig {
    /// Resolve the effective ToolsConfig by merging toolset defaults with
    /// explicit overrides. Explicit tool fields take precedence over the preset.
    fn resolved(&self) -> ToolsConfig {
        let preset = match &self.toolset {
            Some(ts) => ts.expand(),
            None => return self.clone(),
        };

        // For each tool field: use explicit (self) if Some, otherwise use preset
        ToolsConfig {
            toolset: None,
            bash: self.bash.clone().or(preset.bash),
            read: self.read.clone().or(preset.read),
            write: self.write.clone().or(preset.write),
            edit: self.edit.clone().or(preset.edit),
            grep: self.grep.clone().or(preset.grep),
            find: self.find.clone().or(preset.find),
            ls: self.ls.clone().or(preset.ls),
        }
    }

    /// Derive sandbox policy from tool configuration.
    pub fn to_policy(&self) -> ToolPolicy {
        let resolved = self.resolved();
        let mut allowed_binaries = Vec::new();
        let mut allowed_read_paths = Vec::new();
        let mut allowed_write_paths = Vec::new();

        if let Some(bash) = &resolved.bash {
            allowed_binaries.extend(bash.allowed_binaries.clone());
        }
        // grep/find/ls require their standard binaries
        if resolved.grep.is_some() {
            allowed_binaries.push("grep".into());
        }
        if resolved.find.is_some() {
            allowed_binaries.push("find".into());
        }
        if resolved.ls.is_some() {
            allowed_binaries.push("ls".into());
        }

        if let Some(read) = &resolved.read {
            allowed_read_paths.extend(read.allowed_paths.iter().map(|p| expand_tilde(p)));
        }
        if let Some(write) = &resolved.write {
            allowed_write_paths.extend(write.allowed_paths.iter().map(|p| expand_tilde(p)));
        }
        if let Some(edit) = &resolved.edit {
            // edit implies both read and write
            allowed_read_paths.extend(edit.allowed_paths.iter().map(|p| expand_tilde(p)));
            allowed_write_paths.extend(edit.allowed_paths.iter().map(|p| expand_tilde(p)));
        }

        ToolPolicy {
            allowed_binaries,
            allowed_read_paths,
            allowed_write_paths,
        }
    }

    /// Returns the list of enabled tool names.
    pub fn tool_names(&self) -> Vec<String> {
        let resolved = self.resolved();
        let mut names = Vec::new();
        if resolved.bash.is_some() {
            names.push("bash".into());
        }
        if resolved.read.is_some() {
            names.push("read".into());
        }
        if resolved.write.is_some() {
            names.push("write".into());
        }
        if resolved.edit.is_some() {
            names.push("edit".into());
        }
        if resolved.grep.is_some() {
            names.push("grep".into());
        }
        if resolved.find.is_some() {
            names.push("find".into());
        }
        if resolved.ls.is_some() {
            names.push("ls".into());
        }
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEISHU_YAML: &str = r#"
name: feishu-assistant
description: "飞书助手"
llm:
  provider: anthropic
  model: claude-haiku-4-5-20251001
  max_tokens: 4096
system_prompt: |
  你是飞书助手。
tools:
  bash:
    allowed_binaries: [feishu]
  read:
    allowed_paths: ["~/Documents/"]
constraints:
  max_turns: 10
  timeout_secs: 120
"#;

    #[test]
    fn parse_feishu_config() {
        let config: AgentConfig = serde_yaml::from_str(FEISHU_YAML).unwrap();
        assert_eq!(config.name, "feishu-assistant");
        assert_eq!(config.description, "飞书助手");
        assert_eq!(config.llm.provider, "anthropic");
        assert_eq!(config.llm.model, "claude-haiku-4-5-20251001");
        assert_eq!(config.llm.max_tokens, 4096);
        assert!(config.system_prompt.contains("飞书助手"));
        assert!(config.tools.bash.is_some());
        assert!(config.tools.read.is_some());
        assert!(config.tools.write.is_none());
        assert!(config.tools.edit.is_none());
        assert!(config.tools.grep.is_none());
        assert!(config.tools.find.is_none());
        assert!(config.tools.ls.is_none());
        assert_eq!(config.constraints.max_turns, 10);
        assert_eq!(config.constraints.timeout_secs, 120);
    }

    #[test]
    fn parse_all_tools_config() {
        let yaml = r#"
name: full-agent
description: "all tools enabled"
llm:
  provider: openai
  model: gpt-4
  max_tokens: 2048
system_prompt: "test"
tools:
  bash:
    allowed_binaries: [python, node]
  read:
    allowed_paths: ["/home/user/src"]
  write:
    allowed_paths: ["/home/user/src", "/tmp"]
  edit:
    allowed_paths: ["/home/user/src"]
  grep: {}
  find: {}
  ls: {}
constraints:
  max_turns: 20
  timeout_secs: 300
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.tools.bash.is_some());
        assert!(config.tools.read.is_some());
        assert!(config.tools.write.is_some());
        assert!(config.tools.edit.is_some());
        assert!(config.tools.grep.is_some());
        assert!(config.tools.find.is_some());
        assert!(config.tools.ls.is_some());
    }

    #[test]
    fn parse_minimal_tools_config() {
        let yaml = r#"
name: minimal
description: "no tools"
llm:
  provider: anthropic
  model: claude-sonnet-4-20250514
  max_tokens: 1024
system_prompt: "test"
tools: {}
constraints:
  max_turns: 5
  timeout_secs: 60
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.tools.bash.is_none());
        assert!(config.tools.read.is_none());
        assert!(config.tools.write.is_none());
    }

    #[test]
    fn to_policy_bash_only() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  bash:
    allowed_binaries: [python, cargo]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert_eq!(policy.allowed_binaries, vec!["python", "cargo"]);
        assert!(policy.allowed_read_paths.is_empty());
        assert!(policy.allowed_write_paths.is_empty());
    }

    #[test]
    fn to_policy_grep_find_ls_add_binaries() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  grep: {}
  find: {}
  ls: {}
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert!(policy.allowed_binaries.contains(&"grep".to_string()));
        assert!(policy.allowed_binaries.contains(&"find".to_string()));
        assert!(policy.allowed_binaries.contains(&"ls".to_string()));
    }

    #[test]
    fn to_policy_read_write_paths() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  read:
    allowed_paths: ["/home/user/docs"]
  write:
    allowed_paths: ["/tmp"]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert_eq!(policy.allowed_read_paths, vec!["/home/user/docs"]);
        assert_eq!(policy.allowed_write_paths, vec!["/tmp"]);
    }

    #[test]
    fn to_policy_edit_implies_read_and_write() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  edit:
    allowed_paths: ["/src"]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert!(policy.allowed_read_paths.contains(&"/src".to_string()));
        assert!(policy.allowed_write_paths.contains(&"/src".to_string()));
    }

    #[test]
    fn to_policy_combined() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  bash:
    allowed_binaries: [feishu]
  read:
    allowed_paths: ["/docs"]
  edit:
    allowed_paths: ["/src"]
  grep: {}
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        // bash binaries + grep binary
        assert!(policy.allowed_binaries.contains(&"feishu".to_string()));
        assert!(policy.allowed_binaries.contains(&"grep".to_string()));
        // read paths: from read + edit
        assert!(policy.allowed_read_paths.contains(&"/docs".to_string()));
        assert!(policy.allowed_read_paths.contains(&"/src".to_string()));
        // write paths: from edit only
        assert!(policy.allowed_write_paths.contains(&"/src".to_string()));
        assert!(!policy.allowed_write_paths.contains(&"/docs".to_string()));
    }

    #[test]
    fn to_policy_empty_tools() {
        let tools = ToolsConfig {
            toolset: None,
            bash: None,
            read: None,
            write: None,
            edit: None,
            grep: None,
            find: None,
            ls: None,
        };
        let policy = tools.to_policy();
        assert!(policy.allowed_binaries.is_empty());
        assert!(policy.allowed_read_paths.is_empty());
        assert!(policy.allowed_write_paths.is_empty());
    }

    #[test]
    fn to_policy_expands_tilde_in_paths() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  read:
    allowed_paths: ["~/Documents"]
  write:
    allowed_paths: ["~/output"]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        // After expansion, paths should NOT start with "~/"
        for p in &policy.allowed_read_paths {
            assert!(!p.starts_with("~/"), "tilde should be expanded, got: {p}");
        }
        for p in &policy.allowed_write_paths {
            assert!(!p.starts_with("~/"), "tilde should be expanded, got: {p}");
        }
        // Paths should end with the original suffix
        assert!(
            policy.allowed_read_paths[0].ends_with("/Documents")
                || policy.allowed_read_paths[0].ends_with("\\Documents"),
            "expanded path should end with Documents, got: {}",
            policy.allowed_read_paths[0]
        );
    }

    #[test]
    fn to_policy_preserves_absolute_paths() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  read:
    allowed_paths: ["/workspace/src"]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert_eq!(policy.allowed_read_paths[0], "/workspace/src");
    }

    // ========================================================================
    // Phase 5: LlmConfig extensions (base_url, api_key_env)
    // ========================================================================

    #[test]
    fn parse_llm_config_with_base_url_and_api_key_env() {
        let yaml = r#"
name: custom-endpoint
description: "custom LLM endpoint"
llm:
  provider: qwen
  model: qwen-plus
  max_tokens: 4096
  base_url: "https://custom.api.example.com/v1"
  api_key_env: "MY_CUSTOM_KEY"
system_prompt: "test"
tools: {}
constraints: { max_turns: 10, timeout_secs: 120 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.llm.base_url.as_deref(),
            Some("https://custom.api.example.com/v1")
        );
        assert_eq!(config.llm.api_key_env.as_deref(), Some("MY_CUSTOM_KEY"));
    }

    #[test]
    fn parse_llm_config_without_optional_fields() {
        // base_url and api_key_env should default to None when omitted
        let yaml = r#"
name: default
description: "uses defaults"
llm:
  provider: deepseek
  model: deepseek-chat
  max_tokens: 2048
system_prompt: "test"
tools: {}
constraints: { max_turns: 5, timeout_secs: 60 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.llm.base_url.is_none());
        assert!(config.llm.api_key_env.is_none());
    }

    // ========================================================================
    // Phase 5: SandboxConfig
    // ========================================================================

    #[test]
    fn parse_sandbox_config_full() {
        let yaml = r#"
name: sandbox-agent
description: "with sandbox config"
llm: { provider: qwen, model: qwen-plus, max_tokens: 4096 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 10, timeout_secs: 120 }
sandbox:
  cpus: 2
  memory_mib: 1024
  network: true
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.cpus, 2);
        assert_eq!(sb.memory_mib, 1024);
        assert!(sb.network);
    }

    #[test]
    fn parse_sandbox_config_defaults() {
        let yaml = r#"
name: sandbox-defaults
description: "sandbox with defaults"
llm: { provider: qwen, model: qwen-plus, max_tokens: 4096 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 10, timeout_secs: 120 }
sandbox: {}
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.cpus, 1);
        assert_eq!(sb.memory_mib, 512);
        assert!(!sb.network);
    }

    #[test]
    fn parse_no_sandbox_config() {
        // sandbox field omitted entirely — should be None
        let yaml = r#"
name: no-sandbox
description: "no sandbox config"
llm: { provider: qwen, model: qwen-plus, max_tokens: 4096 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 10, timeout_secs: 120 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.sandbox.is_none());
    }

    // ========================================================================
    // Phase 5: tool_names() helper
    // ========================================================================

    #[test]
    fn tool_names_all_enabled() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  bash: { allowed_binaries: [python] }
  read: { allowed_paths: ["/"] }
  write: { allowed_paths: ["/"] }
  edit: { allowed_paths: ["/"] }
  grep: {}
  find: {}
  ls: {}
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 7);
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"read".to_string()));
        assert!(names.contains(&"write".to_string()));
        assert!(names.contains(&"edit".to_string()));
        assert!(names.contains(&"grep".to_string()));
        assert!(names.contains(&"find".to_string()));
        assert!(names.contains(&"ls".to_string()));
    }

    #[test]
    fn tool_names_partial() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  bash: { allowed_binaries: [cargo] }
  grep: {}
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"grep".to_string()));
    }

    #[test]
    fn tool_names_empty() {
        let tools = ToolsConfig {
            toolset: None,
            bash: None,
            read: None,
            write: None,
            edit: None,
            grep: None,
            find: None,
            ls: None,
        };
        assert!(tools.tool_names().is_empty());
    }

    // ========================================================================
    // Phase 5: backward compatibility — existing YAML still parses
    // ========================================================================

    #[test]
    fn existing_feishu_yaml_still_parses_with_new_fields() {
        // The original FEISHU_YAML from Phase 0 must still work
        let config: AgentConfig = serde_yaml::from_str(FEISHU_YAML).unwrap();
        assert_eq!(config.name, "feishu-assistant");
        // New optional fields should be None
        assert!(config.llm.base_url.is_none());
        assert!(config.llm.api_key_env.is_none());
        assert!(config.sandbox.is_none());
    }

    // ========================================================================
    // Phase 5: YAML config examples
    // ========================================================================

    #[test]
    fn parse_coding_assistant_yaml() {
        let yaml = r#"
name: coding-assistant
description: "Qwen-based coding assistant"
llm:
  provider: qwen
  model: qwen-plus
  max_tokens: 8192
  base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1"
system_prompt: |
  你是一个代码助手。帮助用户编写、调试和优化代码。
  工作目录是 /workspace，请在该目录下操作。
tools:
  bash:
    allowed_binaries: [python, python3, pip, cargo, rustc, node, npm, git]
  read:
    allowed_paths: ["/workspace"]
  write:
    allowed_paths: ["/workspace"]
  edit:
    allowed_paths: ["/workspace"]
  grep: {}
  find: {}
  ls: {}
constraints:
  max_turns: 30
  timeout_secs: 600
sandbox:
  cpus: 2
  memory_mib: 2048
  network: true
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.name, "coding-assistant");
        assert_eq!(config.llm.provider, "qwen");
        assert_eq!(config.llm.model, "qwen-plus");
        assert_eq!(config.llm.max_tokens, 8192);
        assert_eq!(
            config.llm.base_url.as_deref(),
            Some("https://dashscope.aliyuncs.com/compatible-mode/v1")
        );
        assert!(config.llm.api_key_env.is_none()); // uses default DASHSCOPE_API_KEY
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.cpus, 2);
        assert_eq!(sb.memory_mib, 2048);
        assert!(sb.network);
        assert_eq!(config.tools.tool_names().len(), 7);
    }

    #[test]
    fn parse_deepseek_coder_yaml() {
        let yaml = r#"
name: deepseek-coder
description: "DeepSeek V3 coding agent"
llm:
  provider: deepseek
  model: deepseek-chat
  max_tokens: 8192
  api_key_env: "DEEPSEEK_API_KEY"
system_prompt: |
  You are a coding assistant powered by DeepSeek.
tools:
  bash:
    allowed_binaries: [python3, cargo, node, git]
  read:
    allowed_paths: ["/workspace"]
  write:
    allowed_paths: ["/workspace"]
  edit:
    allowed_paths: ["/workspace"]
  grep: {}
  find: {}
  ls: {}
constraints:
  max_turns: 25
  timeout_secs: 300
sandbox:
  cpus: 1
  memory_mib: 1024
  network: true
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.name, "deepseek-coder");
        assert_eq!(config.llm.provider, "deepseek");
        assert_eq!(config.llm.api_key_env.as_deref(), Some("DEEPSEEK_API_KEY"));
        assert!(config.llm.base_url.is_none()); // uses default deepseek endpoint
    }

    // ========================================================================
    // P6: Toolset presets
    // ========================================================================

    #[test]
    fn toolset_coding_enables_all_seven_tools() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: coding
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 7);
        for tool in &["bash", "read", "write", "edit", "grep", "find", "ls"] {
            assert!(names.contains(&tool.to_string()), "missing tool: {tool}");
        }
    }

    #[test]
    fn toolset_coding_policy_allows_all_binaries() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: coding
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert!(policy.allowed_binaries.contains(&"*".to_string()));
    }

    #[test]
    fn toolset_readonly_has_no_bash_no_write_no_edit() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: readonly
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert!(!names.contains(&"bash".to_string()));
        assert!(!names.contains(&"write".to_string()));
        assert!(!names.contains(&"edit".to_string()));
        assert!(names.contains(&"read".to_string()));
        assert!(names.contains(&"grep".to_string()));
        assert!(names.contains(&"find".to_string()));
        assert!(names.contains(&"ls".to_string()));
    }

    #[test]
    fn toolset_minimal_same_as_readonly() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: minimal
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 4);
        assert!(!names.contains(&"bash".to_string()));
        assert!(names.contains(&"read".to_string()));
    }

    #[test]
    fn toolset_ops_has_bash_read_grep_find_ls_no_write_edit() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: ops
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 5);
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"read".to_string()));
        assert!(names.contains(&"grep".to_string()));
        assert!(names.contains(&"find".to_string()));
        assert!(names.contains(&"ls".to_string()));
        assert!(!names.contains(&"write".to_string()));
        assert!(!names.contains(&"edit".to_string()));
    }

    #[test]
    fn toolset_ops_policy_has_restricted_binaries() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: ops
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        // ops preset should have specific allowed binaries, not wildcard
        assert!(!policy.allowed_binaries.contains(&"*".to_string()));
        // Should include common ops tools
        for bin in &["curl", "docker", "kubectl", "ssh", "systemctl"] {
            assert!(
                policy.allowed_binaries.contains(&bin.to_string()),
                "ops preset missing binary: {bin}"
            );
        }
    }

    #[test]
    fn toolset_web_has_bash_and_read_only() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: web
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"read".to_string()));
        assert!(!names.contains(&"write".to_string()));
        assert!(!names.contains(&"edit".to_string()));
    }

    #[test]
    fn toolset_web_policy_has_http_binaries() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: web
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert!(!policy.allowed_binaries.contains(&"*".to_string()));
        for bin in &["curl", "wget", "jq", "python"] {
            assert!(
                policy.allowed_binaries.contains(&bin.to_string()),
                "web preset missing binary: {bin}"
            );
        }
    }

    #[test]
    fn toolset_with_explicit_override_uses_override() {
        // toolset provides defaults, but explicit tool config overrides
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: coding
  bash:
    allowed_binaries: [python]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        // explicit bash config overrides toolset's wildcard
        assert!(policy.allowed_binaries.contains(&"python".to_string()));
        assert!(!policy.allowed_binaries.contains(&"*".to_string()));
    }

    #[test]
    fn toolset_with_partial_override_merges() {
        // toolset provides base tools, explicit config overrides specific ones
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: coding
  read:
    allowed_paths: ["/restricted"]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        // All 7 tools still enabled (from toolset)
        assert_eq!(names.len(), 7);
        // But read has restricted paths from explicit config
        let policy = config.tools.to_policy();
        assert_eq!(policy.allowed_read_paths, vec!["/restricted"]);
    }

    #[test]
    fn toolset_unknown_value_returns_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: nonexistent
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let result = serde_yaml::from_str::<AgentConfig>(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn no_toolset_field_backward_compatible() {
        // Existing configs without toolset field should parse exactly as before
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  bash:
    allowed_binaries: [cargo]
  grep: {}
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"grep".to_string()));
    }

    #[test]
    fn empty_tools_with_no_toolset_has_zero_tools() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.tools.tool_names().is_empty());
    }

    #[test]
    fn toolset_coding_readonly_round_trip_serialize() {
        // Toolset should serialize/deserialize correctly
        let coding = Toolset::Coding;
        let readonly = Toolset::Readonly;
        assert_eq!(
            serde_yaml::to_string(&coding).unwrap().trim(),
            "coding"
        );
        assert_eq!(
            serde_yaml::to_string(&readonly).unwrap().trim(),
            "readonly"
        );
    }

    #[test]
    fn toolset_expand_coding_all_tools_present() {
        let expanded = Toolset::Coding.expand();
        assert!(expanded.bash.is_some());
        assert!(expanded.read.is_some());
        assert!(expanded.write.is_some());
        assert!(expanded.edit.is_some());
        assert!(expanded.grep.is_some());
        assert!(expanded.find.is_some());
        assert!(expanded.ls.is_some());
        assert_eq!(expanded.bash.unwrap().allowed_binaries, vec!["*"]);
    }

    #[test]
    fn toolset_expand_ops_has_bash_read_grep_find_ls() {
        let expanded = Toolset::Ops.expand();
        assert!(expanded.bash.is_some());
        assert!(expanded.read.is_some());
        assert!(expanded.write.is_none());
        assert!(expanded.edit.is_none());
        assert!(expanded.grep.is_some());
        assert!(expanded.find.is_some());
        assert!(expanded.ls.is_some());
        let binaries = &expanded.bash.unwrap().allowed_binaries;
        assert!(!binaries.contains(&"*".to_string()));
        assert!(binaries.contains(&"curl".to_string()));
        assert!(binaries.contains(&"docker".to_string()));
    }

    #[test]
    fn toolset_expand_web_has_bash_read_only() {
        let expanded = Toolset::Web.expand();
        assert!(expanded.bash.is_some());
        assert!(expanded.read.is_some());
        assert!(expanded.write.is_none());
        assert!(expanded.edit.is_none());
        assert!(expanded.grep.is_none());
        assert!(expanded.find.is_none());
        assert!(expanded.ls.is_none());
        let binaries = &expanded.bash.unwrap().allowed_binaries;
        assert!(binaries.contains(&"curl".to_string()));
        assert!(binaries.contains(&"python".to_string()));
    }

    #[test]
    fn toolset_expand_minimal_read_grep_find_ls_only() {
        let expanded = Toolset::Minimal.expand();
        assert!(expanded.bash.is_none());
        assert!(expanded.read.is_some());
        assert!(expanded.write.is_none());
        assert!(expanded.edit.is_none());
        assert!(expanded.grep.is_some());
        assert!(expanded.find.is_some());
        assert!(expanded.ls.is_some());
    }

    #[test]
    fn toolset_expand_readonly_same_as_minimal() {
        let minimal = Toolset::Minimal.expand();
        let readonly = Toolset::Readonly.expand();
        assert_eq!(minimal.bash.is_some(), readonly.bash.is_some());
        assert_eq!(minimal.read.is_some(), readonly.read.is_some());
        assert_eq!(minimal.write.is_some(), readonly.write.is_some());
        assert_eq!(minimal.edit.is_some(), readonly.edit.is_some());
        assert_eq!(minimal.grep.is_some(), readonly.grep.is_some());
        assert_eq!(minimal.find.is_some(), readonly.find.is_some());
        assert_eq!(minimal.ls.is_some(), readonly.ls.is_some());
    }

    #[test]
    fn toolset_readonly_policy_precise() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: readonly
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert!(policy.allowed_write_paths.is_empty());
        // readonly has no bash, so only grep/find/ls binaries
        assert!(policy.allowed_binaries.contains(&"grep".to_string()));
        assert!(policy.allowed_binaries.contains(&"find".to_string()));
        assert!(policy.allowed_binaries.contains(&"ls".to_string()));
        assert!(!policy.allowed_binaries.contains(&"*".to_string()));
        assert_eq!(policy.allowed_binaries.len(), 3);
    }

    #[test]
    fn toolset_minimal_policy_same_as_readonly() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: minimal
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert!(policy.allowed_write_paths.is_empty());
        assert_eq!(policy.allowed_binaries.len(), 3);
    }

    #[test]
    fn toolset_web_tool_names_precise() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: web
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"read".to_string()));
        // Explicitly verify these are absent
        assert!(!names.contains(&"write".to_string()));
        assert!(!names.contains(&"edit".to_string()));
        assert!(!names.contains(&"grep".to_string()));
        assert!(!names.contains(&"find".to_string()));
        assert!(!names.contains(&"ls".to_string()));
    }

    // -- Override / merge scenarios --

    #[test]
    fn toolset_readonly_plus_explicit_bash_adds_tool() {
        // "additive" override: readonly base + explicit bash
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: readonly
  bash:
    allowed_binaries: [python3]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        // Should have readonly tools + bash
        assert_eq!(names.len(), 5);
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"read".to_string()));
        let policy = config.tools.to_policy();
        assert!(policy.allowed_binaries.contains(&"python3".to_string()));
    }

    #[test]
    fn toolset_coding_override_write_with_restricted_paths() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: coding
  write:
    allowed_paths: ["/tmp"]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 7); // all tools still present
        let policy = config.tools.to_policy();
        assert_eq!(policy.allowed_write_paths, vec!["/tmp"]);
    }

    #[test]
    fn toolset_coding_override_edit_with_restricted_paths() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: coding
  edit:
    allowed_paths: ["/src"]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        // edit implies read and write paths
        assert!(policy.allowed_read_paths.contains(&"/src".to_string()));
        assert!(policy.allowed_write_paths.contains(&"/src".to_string()));
    }

    #[test]
    fn toolset_ops_override_bash_narrows_binaries() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: ops
  bash:
    allowed_binaries: [docker, kubectl]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        // Only docker + kubectl (from explicit) + grep/find/ls (from preset tools)
        assert!(policy.allowed_binaries.contains(&"docker".to_string()));
        assert!(policy.allowed_binaries.contains(&"kubectl".to_string()));
        assert!(!policy.allowed_binaries.contains(&"curl".to_string()));
    }

    // -- Error / boundary scenarios --

    #[test]
    fn toolset_empty_string_returns_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: ""
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn toolset_uppercase_returns_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: CODING
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn toolset_mixed_case_returns_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: Coding
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn toolset_integer_value_returns_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: 42
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn toolset_list_value_returns_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: [coding, ops]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn toolset_explicit_null_treated_as_no_toolset() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: ~
  bash:
    allowed_binaries: [cargo]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"bash".to_string()));
    }

    #[test]
    fn toolset_serialize_round_trip_all_variants() {
        for (toolset, expected) in [
            (Toolset::Coding, "coding"),
            (Toolset::Ops, "ops"),
            (Toolset::Web, "web"),
            (Toolset::Minimal, "minimal"),
            (Toolset::Readonly, "readonly"),
        ] {
            let serialized = serde_yaml::to_string(&toolset).unwrap();
            assert_eq!(serialized.trim(), expected);
            let deserialized: Toolset = serde_yaml::from_str(expected).unwrap();
            assert_eq!(deserialized, toolset);
        }
    }

    // -- State: "cannot disable preset tool" behavior documentation --

    #[test]
    fn toolset_preset_tool_cannot_be_disabled_via_yaml_null() {
        // This documents the design: when a toolset is set, you cannot
        // disable a tool that the preset enables by setting it to null/~.
        // serde_yaml deserializes `write: ~` as None, and `None.or(preset)`
        // falls back to the preset. This is intentional — use a more
        // restrictive toolset instead.
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: coding
  write: ~
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        // write is still present because toolset preset fills it in
        assert!(names.contains(&"write".to_string()));
        assert_eq!(names.len(), 7);
    }
}
