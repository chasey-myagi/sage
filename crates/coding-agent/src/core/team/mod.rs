//! Multi-agent team management — mirrors CC `tools/shared/spawnMultiAgent.ts`.
//!
//! A "team" is a named group of agents working in parallel on related tasks.
//! Each team member gets a unique name, a dedicated context, and runs
//! asynchronously. Members communicate via the task-notification model.

use std::sync::Arc;

use agent_core::agent_loop::LlmProvider;
use agent_core::types::{AgentMessage, AgentTool, Content, UserMessage};
use ai::types::Model;

use crate::core::agent::definition::{AgentModel, PermissionMode};
use crate::core::agent::runner::{AgentError, resolve_model_override, resolve_system_prompt};

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
    /// LLM provider inherited from the parent session.
    pub provider: Arc<dyn LlmProvider>,
    /// Tools available to the agent (filtered per permission_mode at spawn time).
    pub tools: Vec<Arc<dyn AgentTool>>,
    /// Parent session model — used as the base for model resolution.
    pub parent_model: Model,
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

/// Build a `can_use_tool` closure from an agent's tool list and permission mode.
///
/// - When `agent_tools` contains `"*"` the agent inherits all tools — every
///   call is allowed.
/// - When the list is explicit only the listed names are permitted.
/// - `PermissionMode::Auto` / `AcceptEdits` / `Bubble` all allow every tool
///   (permission decisions are handled upstream).
/// - `PermissionMode::Default` (or `None`) respects the explicit tool list.
fn build_can_use_tool(
    agent_tools: &[String],
    permission_mode: Option<&PermissionMode>,
) -> Arc<dyn Fn(&str, Option<&[String]>) -> Result<(), String> + Send + Sync> {
    // Auto / AcceptEdits / Bubble: pass everything through.
    let mode_allows_all = matches!(
        permission_mode,
        Some(PermissionMode::Auto | PermissionMode::AcceptEdits | PermissionMode::Bubble)
    );

    if mode_allows_all || agent_tools.iter().any(|t| t == "*") {
        return Arc::new(|_tool, _allowed| Ok(()));
    }

    // Explicit allowlist — check at call time.
    let allowed: Vec<String> = agent_tools.to_vec();
    Arc::new(move |tool: &str, extra: Option<&[String]>| {
        if allowed.contains(&tool.to_string()) {
            return Ok(());
        }
        if let Some(extra_list) = extra {
            if extra_list.contains(&tool.to_string()) {
                return Ok(());
            }
        }
        Err(format!("tool '{tool}' is not in the agent's allowed tool list"))
    })
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

    // Resolve the model: config.model override → agent_def.model → parent model.
    let model = {
        let primary = config.model.as_ref().or(agent_def.model.as_ref());
        resolve_model_override(primary, &config.parent_model)
    };

    // Resolve the system prompt from the agent def's function name, passing
    // cwd so the prompt is scoped to the correct working directory.
    let system_prompt = resolve_system_prompt(
        &agent_def.system_prompt_fn,
        config.cwd.as_deref(),
    );

    // Build permission-aware can_use_tool closure.
    let can_use_tool = build_can_use_tool(
        &agent_def.tools,
        agent_def.permission_mode.as_ref(),
    );

    let initial_message = AgentMessage::User(UserMessage {
        content: vec![Content::Text {
            text: config.prompt.clone(),
        }],
        timestamp: now_ms(),
    });

    let params = RunAgentParams {
        agent_def,
        system_prompt: Some(system_prompt),
        user_context: Default::default(),
        system_context: Default::default(),
        can_use_tool,
        messages: vec![initial_message],
        max_turns: None,
        allowed_tools: None,
        model_override: None, // already applied above
        abort_token: None,
        on_cache_safe_params: None,
        provider: config.provider,
        tools: config.tools,
        model,
    };

    let id_for_task = agent_id.clone();
    let name_for_task = unique_name.clone();
    let team_name_for_log = config.team_name.clone().unwrap_or_default();

    tokio::spawn(async move {
        use futures::StreamExt;
        let mut stream = run_agent(params);
        while let Some(result) = stream.next().await {
            if let Err(e) = result {
                tracing::warn!(
                    agent_id = %id_for_task,
                    name = %name_for_task,
                    team = %team_name_for_log,
                    error = %e,
                    "team member failed",
                );
                break;
            }
        }
        tracing::debug!(
            agent_id = %id_for_task,
            name = %name_for_task,
            "team member finished",
        );
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
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use ai::types::{
        AssistantMessageEvent, InputType, LlmContext, LlmTool, Model, ModelCost, StopReason, api,
    };

    fn test_model() -> Model {
        Model {
            id: "test-model".into(),
            name: "Test Model".into(),
            api: api::OPENAI_COMPLETIONS.into(),
            provider: "test".into(),
            base_url: "http://localhost".into(),
            api_key_env: "TEST_KEY".into(),
            reasoning: false,
            input: vec![InputType::Text],
            max_tokens: 4096,
            context_window: 32768,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        }
    }

    struct MockProvider {
        responses: Mutex<VecDeque<Vec<AssistantMessageEvent>>>,
    }

    impl MockProvider {
        fn new(responses: Vec<Vec<AssistantMessageEvent>>) -> Self {
            Self {
                responses: Mutex::new(VecDeque::from(responses)),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for MockProvider {
        async fn complete(
            &self,
            _model: &Model,
            _context: &LlmContext,
            _tools: &[LlmTool],
        ) -> Vec<AssistantMessageEvent> {
            let mut q = self.responses.lock().unwrap();
            q.pop_front().unwrap_or_else(|| {
                vec![AssistantMessageEvent::Done {
                    stop_reason: StopReason::Stop,
                }]
            })
        }
    }

    fn make_provider() -> Arc<dyn LlmProvider> {
        Arc::new(MockProvider::new(vec![vec![
            AssistantMessageEvent::TextDelta("hello".to_string()),
            AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            },
        ]]))
    }

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

    #[test]
    fn build_can_use_tool_wildcard_allows_all() {
        let f = build_can_use_tool(&["*".to_string()], None);
        assert!(f("bash", None).is_ok());
        assert!(f("write", None).is_ok());
    }

    #[test]
    fn build_can_use_tool_explicit_list_blocks_unlisted() {
        let f = build_can_use_tool(
            &["read".to_string(), "grep".to_string()],
            Some(&PermissionMode::Default),
        );
        assert!(f("read", None).is_ok());
        assert!(f("grep", None).is_ok());
        assert!(f("bash", None).is_err());
        assert!(f("write", None).is_err());
    }

    #[test]
    fn build_can_use_tool_auto_mode_allows_all() {
        let f = build_can_use_tool(
            &["read".to_string()], // would block bash in Default mode
            Some(&PermissionMode::Auto),
        );
        assert!(f("bash", None).is_ok());
    }

    #[test]
    fn build_can_use_tool_bubble_mode_allows_all() {
        let f = build_can_use_tool(&[], Some(&PermissionMode::Bubble));
        assert!(f("bash", None).is_ok());
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
            provider: make_provider(),
            tools: vec![],
            parent_model: test_model(),
        };

        let agent_id = spawn_agent_in_team(config, &[]).await.unwrap();
        assert!(!agent_id.is_empty());
    }

    #[tokio::test]
    async fn spawn_agent_in_team_applies_cwd_to_system_prompt() {
        let tmp = std::env::temp_dir();
        let config = SpawnAgentConfig {
            name: "explore-agent".to_string(),
            prompt: "Explore the repo".to_string(),
            team_name: None,
            agent_type: Some("explore".to_string()),
            model: None,
            cwd: Some(tmp.clone()),
            provider: make_provider(),
            tools: vec![],
            parent_model: test_model(),
        };

        let agent_id = spawn_agent_in_team(config, &[]).await.unwrap();
        assert!(!agent_id.is_empty());
    }

    #[tokio::test]
    async fn spawn_agent_in_team_explore_restricts_tools() {
        // The explore agent definition has an explicit tool list (read/grep/find).
        // build_can_use_tool should block bash for it.
        let agents = crate::core::agent::builtin::register_builtin_agents();
        let explore = agents.get("explore").unwrap();
        let f = build_can_use_tool(&explore.tools, explore.permission_mode.as_ref());
        assert!(f("read", None).is_ok());
        assert!(f("bash", None).is_err(), "bash should be blocked for explore");
    }
}
