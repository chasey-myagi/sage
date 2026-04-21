//! Agent definition types — mirrors CC `tools/AgentTool/loadAgentsDir.ts`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Agent permission mode — mirrors CC's `PermissionMode`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    Auto,
    AcceptEdits,
    /// Permission prompts bubble up to the parent agent.
    Bubble,
}

/// Model selection for a sub-agent.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum AgentModel {
    Opus,
    Sonnet,
    Haiku,
    /// Inherit the parent agent's model at runtime.
    Inherit,
    Custom(String),
}

/// Where this agent definition originates.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentSource {
    BuiltIn,
    Custom,
    Plugin,
}

/// MCP server specification inside an agent definition.
///
/// Can be a string reference to an existing server by name, or an inline
/// definition as a key-value map.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MCPServerSpec {
    /// Reference an existing MCP server by name.
    Reference(String),
    /// Inline server config as `{ name: config }`.
    Inline(HashMap<String, serde_json::Value>),
}

/// Agent definition — mirrors CC's `AgentDefinition` interface.
///
/// Declared in YAML/JSON files or registered programmatically for built-ins.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentDef {
    /// Unique identifier used in tool calls (`subagent_type`).
    pub agent_type: String,
    /// Human-readable description of when to use this agent.
    pub when_to_use: String,
    /// Allowed tool names. `["*"]` means inherit all parent tools.
    pub tools: Vec<String>,
    /// Maximum number of turns before the agent stops.
    pub max_turns: Option<u32>,
    /// Model to run the agent with. `None` means use the session default.
    pub model: Option<AgentModel>,
    pub permission_mode: Option<PermissionMode>,
    pub source: AgentSource,
    /// Optional MCP server specs specific to this agent.
    pub mcp_servers: Option<Vec<MCPServerSpec>>,
    /// Identifier for the function that produces this agent's system prompt.
    pub system_prompt_fn: String,
}

/// Loads and stores agent definitions indexed by `agent_type`.
pub struct AgentLoader {
    agents: HashMap<String, AgentDef>,
}

impl AgentLoader {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }

    /// Populate with built-in agent definitions.
    pub fn load_builtin(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.agents
            .extend(super::builtin::register_builtin_agents());
        Ok(())
    }

    /// Scan a directory for YAML/JSON agent definition files.
    pub fn load_custom<P: AsRef<std::path::Path>>(
        &mut self,
        dir: P,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = dir.as_ref();
        if !dir.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "yaml" || ext == "yml" || ext == "json" {
                let content = std::fs::read_to_string(&path)?;
                let def: AgentDef = if ext == "json" {
                    serde_json::from_str(&content)?
                } else {
                    serde_yaml::from_str(&content)?
                };
                self.agents.insert(def.agent_type.clone(), def);
            }
        }
        Ok(())
    }

    pub fn get(&self, agent_type: &str) -> Option<&AgentDef> {
        self.agents.get(agent_type)
    }

    pub fn list_all(&self) -> Vec<&AgentDef> {
        self.agents.values().collect()
    }

    pub fn insert(&mut self, def: AgentDef) {
        self.agents.insert(def.agent_type.clone(), def);
    }
}

impl Default for AgentLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_def_serialization_round_trip() {
        let def = AgentDef {
            agent_type: "test-agent".to_string(),
            when_to_use: "Testing".to_string(),
            tools: vec!["*".to_string()],
            max_turns: Some(200),
            model: Some(AgentModel::Opus),
            permission_mode: None,
            source: AgentSource::BuiltIn,
            mcp_servers: None,
            system_prompt_fn: "get_test_prompt".to_string(),
        };

        let json = serde_json::to_string(&def).unwrap();
        let parsed: AgentDef = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.agent_type, "test-agent");
        assert_eq!(parsed.model, Some(AgentModel::Opus));
        assert_eq!(parsed.source, AgentSource::BuiltIn);
    }

    #[test]
    fn agent_loader_load_builtin() {
        let mut loader = AgentLoader::new();
        loader.load_builtin().unwrap();
        assert!(loader.get("general-purpose").is_some());
        assert!(loader.get("explore").is_some());
        assert!(loader.get("plan").is_some());
        assert!(loader.get("fork").is_some());
    }

    #[test]
    fn agent_loader_list_all_returns_all_loaded() {
        let mut loader = AgentLoader::new();
        loader.load_builtin().unwrap();
        let all = loader.list_all();
        assert!(!all.is_empty());
    }

    #[test]
    fn agent_loader_get_unknown_returns_none() {
        let loader = AgentLoader::new();
        assert!(loader.get("nonexistent").is_none());
    }
}
