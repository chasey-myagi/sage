//! Built-in agent registration — mirrors CC `tools/AgentTool/builtInAgents.ts`
//! and `tools/AgentTool/built-in/`.

use super::definition::{AgentDef, AgentModel, AgentPermissionMode, AgentSource};
use std::collections::HashMap;

/// Register all built-in agent definitions.
///
/// Returns a map from `agent_type` to `AgentDef`. These mirror the six built-in
/// agents shipped with Claude Code.
pub fn register_builtin_agents() -> HashMap<String, AgentDef> {
    let mut agents = HashMap::new();

    // General Purpose Agent — mirrors `generalPurposeAgent.ts`
    agents.insert(
        "general-purpose".to_string(),
        AgentDef {
            agent_type: "general-purpose".to_string(),
            when_to_use:
                "General-purpose agent for researching complex questions, searching for code, \
                 and executing multi-step tasks."
                    .to_string(),
            tools: vec!["*".to_string()],
            max_turns: Some(200),
            model: Some(AgentModel::Opus),
            permission_mode: None,
            source: AgentSource::BuiltIn,
            mcp_servers: None,
            system_prompt_fn: "get_general_purpose_prompt".to_string(),
        },
    );

    // Plan Agent — mirrors `planAgent.ts`
    agents.insert(
        "plan".to_string(),
        AgentDef {
            agent_type: "plan".to_string(),
            when_to_use:
                "Software architect agent for designing implementation plans. Use when you need \
                 to plan the implementation strategy for a task."
                    .to_string(),
            tools: vec!["read".to_string(), "grep".to_string(), "find".to_string()],
            max_turns: Some(50),
            model: Some(AgentModel::Sonnet),
            permission_mode: None,
            source: AgentSource::BuiltIn,
            mcp_servers: None,
            system_prompt_fn: "get_plan_prompt".to_string(),
        },
    );

    // Explore Agent — mirrors `exploreAgent.ts`
    agents.insert(
        "explore".to_string(),
        AgentDef {
            agent_type: "explore".to_string(),
            when_to_use:
                "Fast agent specialized for exploring codebases. Use when you need to quickly \
                 find files by patterns, search code for keywords, or answer questions about \
                 the codebase."
                    .to_string(),
            tools: vec!["find".to_string(), "grep".to_string(), "read".to_string()],
            max_turns: Some(30),
            model: Some(AgentModel::Haiku),
            permission_mode: None,
            source: AgentSource::BuiltIn,
            mcp_servers: None,
            system_prompt_fn: "get_explore_prompt".to_string(),
        },
    );

    // Verification Agent — mirrors `verificationAgent.ts`
    agents.insert(
        "verification".to_string(),
        AgentDef {
            agent_type: "verification".to_string(),
            when_to_use:
                "Verification agent for confirming implementations are correct. Use after \
                 completing a task to double-check the work."
                    .to_string(),
            tools: vec!["read".to_string(), "grep".to_string(), "bash".to_string()],
            max_turns: Some(50),
            model: Some(AgentModel::Sonnet),
            permission_mode: None,
            source: AgentSource::BuiltIn,
            mcp_servers: None,
            system_prompt_fn: "get_verification_prompt".to_string(),
        },
    );

    // Code Guide Agent — mirrors `claudeCodeGuideAgent.ts`
    agents.insert(
        "code-guide".to_string(),
        AgentDef {
            agent_type: "code-guide".to_string(),
            when_to_use:
                "Code guide agent for understanding unfamiliar codebases. Use when you need \
                 a guided tour of a new project."
                    .to_string(),
            tools: vec!["read".to_string(), "grep".to_string(), "find".to_string()],
            max_turns: Some(30),
            model: Some(AgentModel::Haiku),
            permission_mode: None,
            source: AgentSource::BuiltIn,
            mcp_servers: None,
            system_prompt_fn: "get_code_guide_prompt".to_string(),
        },
    );

    // Fork Agent — synthetic, implicit fork; not selectable via subagent_type.
    // Mirrors CC's `FORK_AGENT` constant in `forkSubagent.ts`.
    agents.insert(
        "fork".to_string(),
        AgentDef {
            agent_type: "fork".to_string(),
            when_to_use: "Implicit fork — inherits full conversation context. Not selectable via \
                 subagent_type; triggered by omitting subagent_type when the fork experiment \
                 is active."
                .to_string(),
            tools: vec!["*".to_string()],
            max_turns: Some(200),
            model: Some(AgentModel::Inherit),
            permission_mode: Some(AgentPermissionMode::Bubble),
            source: AgentSource::BuiltIn,
            mcp_servers: None,
            system_prompt_fn: "get_fork_prompt".to_string(),
        },
    );

    agents
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_returns_six_agents() {
        let agents = register_builtin_agents();
        assert_eq!(agents.len(), 6);
    }

    #[test]
    fn general_purpose_agent_inherits_all_tools() {
        let agents = register_builtin_agents();
        let gp = agents.get("general-purpose").unwrap();
        assert_eq!(gp.tools, vec!["*"]);
    }

    #[test]
    fn fork_agent_uses_inherit_model_and_bubble_permission() {
        let agents = register_builtin_agents();
        let fork = agents.get("fork").unwrap();
        assert_eq!(fork.model, Some(AgentModel::Inherit));
        assert_eq!(fork.permission_mode, Some(AgentPermissionMode::Bubble));
    }

    #[test]
    fn explore_agent_uses_haiku() {
        let agents = register_builtin_agents();
        let explore = agents.get("explore").unwrap();
        assert_eq!(explore.model, Some(AgentModel::Haiku));
    }

    #[test]
    fn all_agents_have_nonempty_when_to_use() {
        let agents = register_builtin_agents();
        for (name, def) in &agents {
            assert!(
                !def.when_to_use.is_empty(),
                "agent '{name}' has empty when_to_use"
            );
        }
    }
}
