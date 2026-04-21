//! Multi-agent team management — mirrors CC `tools/shared/spawnMultiAgent.ts`.
//!
//! A "team" is a named group of agents working in parallel on related tasks.
//! Each team member gets a unique name, a dedicated context, and runs
//! asynchronously. Members communicate via the task-notification model.

use std::sync::Arc;

use agent_core::types::{AgentMessage, Content, UserMessage};

use crate::core::agent::definition::AgentModel;
use crate::core::agent::runner::AgentError;

/// Status of a team member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberStatus {
    Idle,
    Running,
    Done,
    Failed,
}

/// A member of an agent team.
#[derive(Debug, Clone)]
pub struct TeamMember {
    /// Unique identifier for this member (UUID string).
    pub agent_id: String,
    /// Agent type (e.g. "general-purpose", "explore").
    pub agent_type: String,
    /// Human-readable name (unique within the team).
    pub name: String,
    /// Current execution status.
    pub status: MemberStatus,
}

/// Configuration for a team.
#[derive(Debug, Clone)]
pub struct TeamConfig {
    pub name: String,
    pub members: Vec<TeamMember>,
}

/// Configuration for spawning a new agent into a team.
///
/// Mirrors CC's `SpawnTeammateConfig`.
pub struct SpawnAgentConfig {
    /// Base name for the agent. Suffixed with a number if already taken.
    pub name: String,
    /// Initial user prompt for the agent.
    pub prompt: String,
    /// Team this agent belongs to. `None` for standalone agents.
    pub team_name: Option<String>,
    /// Agent type to use. `None` defaults to `"general-purpose"`.
    pub agent_type: Option<String>,
    /// Model override. `None` or `AgentModel::Inherit` inherits the parent's model.
    pub model: Option<AgentModel>,
    /// Working directory for the agent. `None` inherits the session directory.
    pub cwd: Option<std::path::PathBuf>,
}

/// Generate a unique team member name by appending a numeric suffix when the
/// base name is already taken.
///
/// Mirrors CC's `generateUniqueTeammateName`.
pub fn generate_unique_name(base_name: &str, existing_names: &[&str]) -> String {
    let lower: Vec<String> = existing_names.iter().map(|n| n.to_lowercase()).collect();

    if !lower.contains(&base_name.to_lowercase()) {
        return base_name.to_string();
    }

    for suffix in 2u32..=100 {
        let candidate = format!("{base_name}-{suffix}");
        if !lower.contains(&candidate.to_lowercase()) {
            return candidate;
        }
    }

    // Fallback: append a timestamp-based suffix (should not happen in practice)
    format!("{base_name}-{}", ulid::Ulid::new())
}

/// Spawn an agent as a team member and run it in the background.
///
/// Returns the unique agent ID of the spawned member.
///
/// Mirrors CC's `spawnMultiAgent` / `startInProcessTeammate` path.
pub async fn spawn_agent_in_team(
    config: SpawnAgentConfig,
    existing_members: &[&str],
) -> Result<String, AgentError> {
    use crate::core::agent::builtin::register_builtin_agents;
    use crate::core::agent::runner::{RunAgentParams, run_agent};

    let unique_name = generate_unique_name(&config.name, existing_members);
    let agent_id = ulid::Ulid::new().to_string();

    let agents = register_builtin_agents();
    let agent_type = config.agent_type.as_deref().unwrap_or("general-purpose");
    let agent_def = agents
        .get(agent_type)
        .or_else(|| agents.get("general-purpose"))
        .ok_or_else(|| AgentError::ToolNotFound(agent_type.to_string()))?
        .clone();

    let initial_message = AgentMessage::User(UserMessage {
        content: vec![Content::Text {
            text: config.prompt.clone(),
        }],
        timestamp: now_ms(),
    });

    let params = RunAgentParams {
        agent_def,
        system_prompt: None,
        user_context: Default::default(),
        system_context: Default::default(),
        can_use_tool: Arc::new(|_tool, _allowed| Ok(())),
        messages: vec![initial_message],
        max_turns: None,
        allowed_tools: None,
        model_override: config.model,
        abort_token: None,
        on_cache_safe_params: None,
    };

    // Run the agent in a background task. Errors are logged but do not
    // propagate to the caller; the caller tracks status via team file or
    // task-notification events.
    let id_for_task = agent_id.clone();
    let name_for_task = unique_name.clone();
    tokio::spawn(async move {
        use futures::StreamExt;
        let mut stream = run_agent(params);
        while let Some(result) = stream.next().await {
            if let Err(e) = result {
                tracing::warn!(
                    agent_id = %id_for_task,
                    name = %name_for_task,
                    error = %e,
                    "team member failed",
                );
                break;
            }
        }
    });

    Ok(agent_id)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_unique_name_no_conflict() {
        let name = generate_unique_name("researcher", &[]);
        assert_eq!(name, "researcher");
    }

    #[test]
    fn generate_unique_name_conflict_appends_suffix() {
        let name = generate_unique_name("researcher", &["researcher"]);
        assert_eq!(name, "researcher-2");
    }

    #[test]
    fn generate_unique_name_multiple_conflicts() {
        let existing = &["researcher", "researcher-2", "researcher-3"];
        let name = generate_unique_name("researcher", existing);
        assert_eq!(name, "researcher-4");
    }

    #[test]
    fn generate_unique_name_case_insensitive() {
        let name = generate_unique_name("Researcher", &["researcher"]);
        assert_eq!(name, "Researcher-2");
    }

    #[test]
    fn member_status_eq() {
        assert_eq!(MemberStatus::Idle, MemberStatus::Idle);
        assert_ne!(MemberStatus::Running, MemberStatus::Done);
    }

    #[test]
    fn team_config_construction() {
        let team = TeamConfig {
            name: "my-team".to_string(),
            members: vec![TeamMember {
                agent_id: "agent-1".to_string(),
                agent_type: "general-purpose".to_string(),
                name: "researcher".to_string(),
                status: MemberStatus::Idle,
            }],
        };
        assert_eq!(team.name, "my-team");
        assert_eq!(team.members.len(), 1);
    }

    #[tokio::test]
    async fn spawn_agent_in_team_returns_agent_id() {
        let config = SpawnAgentConfig {
            name: "helper".to_string(),
            prompt: "Analyze the codebase".to_string(),
            team_name: Some("test-team".to_string()),
            agent_type: None,
            model: None,
            cwd: None,
        };

        let agent_id = spawn_agent_in_team(config, &[]).await.unwrap();
        assert!(!agent_id.is_empty());
    }
}
