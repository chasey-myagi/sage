use serde::{Deserialize, Serialize};

use crate::tools::ToolPolicy;

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

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolsConfig {
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

#[derive(Debug, Serialize, Deserialize)]
pub struct BashToolConfig {
    pub allowed_binaries: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PathToolConfig {
    pub allowed_paths: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EmptyToolConfig {}

#[derive(Debug, Serialize, Deserialize)]
pub struct Constraints {
    pub max_turns: u32,
    pub timeout_secs: u32,
}

impl ToolsConfig {
    /// Derive sandbox policy from tool configuration.
    pub fn to_policy(&self) -> ToolPolicy {
        let mut allowed_binaries = Vec::new();
        let mut allowed_read_paths = Vec::new();
        let mut allowed_write_paths = Vec::new();

        if let Some(bash) = &self.bash {
            allowed_binaries.extend(bash.allowed_binaries.clone());
        }
        // grep/find/ls require their standard binaries
        if self.grep.is_some() {
            allowed_binaries.push("grep".into());
        }
        if self.find.is_some() {
            allowed_binaries.push("find".into());
        }
        if self.ls.is_some() {
            allowed_binaries.push("ls".into());
        }

        if let Some(read) = &self.read {
            allowed_read_paths.extend(read.allowed_paths.clone());
        }
        if let Some(write) = &self.write {
            allowed_write_paths.extend(write.allowed_paths.clone());
        }
        if let Some(edit) = &self.edit {
            // edit implies both read and write
            allowed_read_paths.extend(edit.allowed_paths.clone());
            allowed_write_paths.extend(edit.allowed_paths.clone());
        }

        ToolPolicy {
            allowed_binaries,
            allowed_read_paths,
            allowed_write_paths,
        }
    }

    /// Returns the list of enabled tool names.
    pub fn tool_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        if self.bash.is_some() {
            names.push("bash".into());
        }
        if self.read.is_some() {
            names.push("read".into());
        }
        if self.write.is_some() {
            names.push("write".into());
        }
        if self.edit.is_some() {
            names.push("edit".into());
        }
        if self.grep.is_some() {
            names.push("grep".into());
        }
        if self.find.is_some() {
            names.push("find".into());
        }
        if self.ls.is_some() {
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
        assert_eq!(config.llm.base_url.as_deref(), Some("https://custom.api.example.com/v1"));
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
        assert_eq!(config.llm.base_url.as_deref(), Some("https://dashscope.aliyuncs.com/compatible-mode/v1"));
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
}
