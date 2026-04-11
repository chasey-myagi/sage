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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LlmConfig {
    pub provider: String,
    pub model: String,
    pub max_tokens: u32,
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
}
