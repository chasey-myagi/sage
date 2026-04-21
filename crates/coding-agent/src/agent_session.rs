//! Agent session wiring — connects the CLI to `agent-core` and the `ai` provider registry.
//!
//! `run_agent_session` is the concrete entry point for print-mode execution:
//! it resolves a model, creates an LLM provider, wires up the default tools,
//! subscribes to events, sends the user message, and waits for completion.

use std::io::Write as _;
use std::sync::{Arc, Mutex};

use agent_core::agent::{Agent, AgentOptions};
use agent_core::agent_loop::LlmProvider;
use agent_core::mcp::{McpClient, McpServerConfig};
use agent_core::tools::backend::LocalBackend;
use agent_core::tools::mcp_tool::discover_mcp_tools;
use agent_core::tools::{AgentTool as SimpleTool, ToolOutput, create_tool};
use agent_core::types::{AgentToolResult, Content, OnUpdateFn};
use ai::registry::{ApiProviderRegistry, StreamOptions};
use ai::types::{AssistantMessageEvent, InputType, Model, ModelCost, Usage};
use crate::config::{CONFIG_DIR_NAME, get_agent_dir, get_sessions_dir};
use crate::core::agent::runner::AgentError;
use crate::core::hooks::executor::HookExecutor;
use crate::core::hooks::runner::HookRunner;
use crate::core::hooks::HooksLifecycle;
use crate::core::settings_manager::SettingsManager;
use crate::core::team::{SpawnAgentConfig, spawn_agent_in_team};
use crate::core::tools::plan_mode::{
    ENTER_PLAN_MODE_TOOL_NAME, EXIT_PLAN_MODE_TOOL_NAME, ExitPlanModeInput, PlanExitStrategy,
    enter_plan_mode, exit_plan_mode,
};
use crate::utils::permissions::{
    PermissionBehavior, PermissionDecision, PermissionMode, PermissionRuleSource,
    ToolPermissionContext,
};
use crate::utils::permissions::engine::check_tool_permission;
use crate::utils::permissions::loader::{load_permissions_from_file, permissions_json_to_rules};
use crate::utils::permissions::parser::permission_rule_value_to_string;

/// Events sent through the interactive-mode channel.
#[derive(Debug, Clone)]
pub enum AgentDelta {
    /// A streaming text fragment.
    Text(String),
    /// Token usage snapshot after a turn completes.
    TurnUsage {
        usage: Usage,
        model: String,
        is_fast: bool,
    },
    /// A tool call has started.
    ToolStart { name: String, args_preview: String },
    /// A tool call has completed.
    ToolEnd {
        name: String,
        success: bool,
        output_preview: String,
    },
    /// A fatal agent error.
    Error(String),
}

// ── Registry-backed LLM provider adapter ────────────────────────────────────

/// Adapts `ai::registry::ApiProviderRegistry` + `StreamOptions` into the
/// `agent_core::agent_loop::LlmProvider` trait.
struct RegistryProvider {
    registry: Arc<ApiProviderRegistry>,
    options: StreamOptions,
}

#[async_trait::async_trait]
impl LlmProvider for RegistryProvider {
    async fn complete(
        &self,
        model: &Model,
        context: &ai::types::LlmContext,
        tools: &[ai::types::LlmTool],
    ) -> Vec<AssistantMessageEvent> {
        let provider = match self.registry.get(&model.api) {
            Some(p) => p,
            None => {
                return vec![AssistantMessageEvent::Error(format!(
                    "No provider registered for API: {}",
                    model.api
                ))];
            }
        };
        provider.stream(model, context, tools, &self.options).await
    }
}

// ── Tool adapter: tools::AgentTool → types::AgentTool ───────────────────────

/// Wraps a `agent_core::tools::AgentTool` (simple interface) into the
/// `agent_core::types::AgentTool` trait (full interface expected by `Agent`).
///
/// Calls `PermissionEngine::check()` before each tool execution, returning a
/// denial message if the tool is blocked by settings.json permission rules.
struct ToolAdapter {
    inner: Box<dyn SimpleTool>,
    tool_description: String,
    tool_schema: serde_json::Value,
    permission_ctx: Arc<Mutex<ToolPermissionContext>>,
}

impl ToolAdapter {
    fn new(tool: Box<dyn SimpleTool>, permission_ctx: Arc<Mutex<ToolPermissionContext>>) -> Self {
        let description = tool.description().to_string();
        let schema = tool.parameters_schema();
        Self {
            inner: tool,
            tool_description: description,
            tool_schema: schema,
            permission_ctx,
        }
    }
}

#[async_trait::async_trait]
impl agent_core::types::AgentTool for ToolAdapter {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn label(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.tool_schema.clone()
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        args: serde_json::Value,
        _signal: Option<tokio_util::sync::CancellationToken>,
        _on_update: Option<&OnUpdateFn>,
    ) -> AgentToolResult {
        // Check permission rules from settings.json before executing.
        let decision = {
            let ctx = self.permission_ctx.lock().unwrap();
            check_tool_permission(&ctx, self.inner.name())
        };
        if let PermissionDecision::Deny { message, .. } = decision {
            return AgentToolResult {
                content: vec![Content::Text {
                    text: format!("Permission denied: {message}"),
                }],
                details: serde_json::Value::Null,
            };
        }

        let output: ToolOutput = self.inner.execute(args).await;
        AgentToolResult {
            content: output.content,
            details: serde_json::Value::Null,
        }
    }
}

// ── Plan-mode tools ──────────────────────────────────────────────────────────

/// Implements the EnterPlanMode tool for the agent loop.
///
/// Transitions the shared permission context into `PermissionMode::Plan`,
/// signalling to the LLM that it should explore and plan without writing files.
struct EnterPlanModeTool {
    permission_ctx: Arc<Mutex<ToolPermissionContext>>,
}

#[async_trait::async_trait]
impl agent_core::types::AgentTool for EnterPlanModeTool {
    fn name(&self) -> &str {
        ENTER_PLAN_MODE_TOOL_NAME
    }

    fn label(&self) -> &str {
        ENTER_PLAN_MODE_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Switches into plan mode, a read-only exploration and design phase. While in plan \
         mode you must not create or edit files. Use this to think through an approach \
         before implementing it. When ready, call ExitPlanMode with your plan."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        _args: serde_json::Value,
        _signal: Option<tokio_util::sync::CancellationToken>,
        _on_update: Option<&OnUpdateFn>,
    ) -> AgentToolResult {
        let mut ctx = self.permission_ctx.lock().unwrap();
        match enter_plan_mode(&mut ctx, false, false) {
            Ok(output) => AgentToolResult {
                content: vec![Content::Text { text: output.message }],
                details: serde_json::Value::Null,
            },
            Err(e) => AgentToolResult {
                content: vec![Content::Text { text: e }],
                details: serde_json::Value::Null,
            },
        }
    }
}

/// Implements the ExitPlanMode tool for the agent loop.
///
/// Restores the previous permission mode and presents the plan for approval.
struct ExitPlanModeTool {
    permission_ctx: Arc<Mutex<ToolPermissionContext>>,
}

#[async_trait::async_trait]
impl agent_core::types::AgentTool for ExitPlanModeTool {
    fn name(&self) -> &str {
        EXIT_PLAN_MODE_TOOL_NAME
    }

    fn label(&self) -> &str {
        EXIT_PLAN_MODE_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Exit plan mode and present your implementation plan for approval. \
         Provide a `plan` parameter with the full plan text. \
         The agent will not proceed with implementation until the plan is approved."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "plan": {
                    "type": "string",
                    "description": "The implementation plan to present for approval"
                }
            },
            "required": ["plan"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        args: serde_json::Value,
        _signal: Option<tokio_util::sync::CancellationToken>,
        _on_update: Option<&OnUpdateFn>,
    ) -> AgentToolResult {
        let plan = args
            .get("plan")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mut ctx = self.permission_ctx.lock().unwrap();
        let input = ExitPlanModeInput { plan };
        match exit_plan_mode(&mut ctx, input, PlanExitStrategy::Standard) {
            Ok(output) => AgentToolResult {
                content: vec![Content::Text { text: output.message }],
                details: serde_json::Value::Null,
            },
            Err(e) => AgentToolResult {
                content: vec![Content::Text { text: e }],
                details: serde_json::Value::Null,
            },
        }
    }
}

// ── Permission context builder ───────────────────────────────────────────────

/// Build a `ToolPermissionContext` from the CLI permission mode and settings files.
///
/// Loads allow/deny/ask rules from the global settings file (UserSettings source)
/// and the project settings file (ProjectSettings source), then applies the
/// requested permission mode.
fn build_permission_context(
    permission_mode: &str,
    cwd: &std::path::Path,
) -> Arc<Mutex<ToolPermissionContext>> {
    let mode = PermissionMode::from_str_lossy(permission_mode);
    let mut ctx = ToolPermissionContext::new(mode);

    let global_settings_path = get_agent_dir().join("settings.json");
    if let Some(perms) = load_permissions_from_file(&global_settings_path) {
        for rule in permissions_json_to_rules(&perms, PermissionRuleSource::UserSettings) {
            let rule_str = permission_rule_value_to_string(&rule.rule_value);
            match rule.rule_behavior {
                PermissionBehavior::Allow => ctx.add_allow_rule(rule.source, rule_str),
                PermissionBehavior::Deny => ctx.add_deny_rule(rule.source, rule_str),
                PermissionBehavior::Ask => ctx.add_ask_rule(rule.source, rule_str),
            }
        }
    }

    let project_settings_path = cwd.join(CONFIG_DIR_NAME).join("settings.json");
    if let Some(perms) = load_permissions_from_file(&project_settings_path) {
        for rule in permissions_json_to_rules(&perms, PermissionRuleSource::ProjectSettings) {
            let rule_str = permission_rule_value_to_string(&rule.rule_value);
            match rule.rule_behavior {
                PermissionBehavior::Allow => ctx.add_allow_rule(rule.source, rule_str),
                PermissionBehavior::Deny => ctx.add_deny_rule(rule.source, rule_str),
                PermissionBehavior::Ask => ctx.add_ask_rule(rule.source, rule_str),
            }
        }
    }

    Arc::new(Mutex::new(ctx))
}

// ── Model construction ───────────────────────────────────────────────────────

/// Build a `Model` from a provider spec + optional model id override.
///
/// Defaults: `anthropic` provider, `claude-sonnet-4-5` model.
fn build_model(provider_id: Option<&str>, model_id: Option<&str>) -> anyhow::Result<Model> {
    let provider_id = provider_id.unwrap_or("anthropic");
    let model_id = model_id.unwrap_or("claude-sonnet-4-5");

    let spec = ai::provider_specs::resolve_provider(provider_id).ok_or_else(|| {
        anyhow::anyhow!(
            "Unknown provider '{}'. Run with --list-models to see available providers.",
            provider_id
        )
    })?;

    Ok(Model {
        id: model_id.to_string(),
        name: model_id.to_string(),
        api: spec.api_kind.to_string(),
        provider: provider_id.to_string(),
        base_url: spec.base_url.to_string(),
        api_key_env: spec.api_key_env.to_string(),
        reasoning: false,
        input: vec![InputType::Text],
        max_tokens: spec.default_max_tokens,
        context_window: spec.default_context_window,
        cost: ModelCost {
            input_per_million: 0.0,
            output_per_million: 0.0,
            cache_read_per_million: 0.0,
            cache_write_per_million: 0.0,
        },
        headers: vec![],
        compat: None,
    })
}

// ── SpawnSubagentTool ────────────────────────────────────────────────────────

struct SpawnSubagentTool;

#[async_trait::async_trait]
impl SimpleTool for SpawnSubagentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Spawn a sub-agent to handle a task asynchronously in the background. \
         The sub-agent has access to the full set of coding tools and runs \
         independently. Returns the unique agent ID of the spawned agent. \
         Use this to parallelise independent sub-tasks or to delegate \
         specialised work to a background worker."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The prompt or task description for the sub-agent to execute."
                },
                "model": {
                    "type": "string",
                    "description": "Optional model ID override (e.g. 'claude-opus-4-5'). Inherits the session default when omitted."
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory for the sub-agent. Defaults to the current directory when omitted."
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> ToolOutput {
        let task = match args.get("task").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => {
                return ToolOutput {
                    content: vec![Content::Text {
                        text: "Error: 'task' parameter is required".to_string(),
                    }],
                    is_error: true,
                };
            }
        };

        let model_id = args.get("model").and_then(|v| v.as_str()).map(str::to_string);
        let cwd = args
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from);

        match spawn_subagent(task, None, model_id, None, None, None, cwd).await {
            Ok(agent_id) => ToolOutput {
                content: vec![Content::Text {
                    text: format!("Sub-agent spawned with ID: {agent_id}"),
                }],
                is_error: false,
            },
            Err(e) => ToolOutput {
                content: vec![Content::Text {
                    text: format!("Failed to spawn sub-agent: {e}"),
                }],
                is_error: true,
            },
        }
    }
}

// ── Default tool list ────────────────────────────────────────────────────────

/// Create the default coding-agent tools backed by the local filesystem.
///
/// All standard tools are wrapped in `ToolAdapter` which enforces permission
/// rules from settings.json before each execution.  `EnterPlanMode`,
/// `ExitPlanMode`, and `SpawnSubagent` are also registered so the LLM can use
/// plan mode and delegate sub-tasks.
fn create_default_tools(
    backend: Arc<LocalBackend>,
    permission_ctx: Arc<Mutex<ToolPermissionContext>>,
) -> Vec<Arc<dyn agent_core::types::AgentTool>> {
    let mut tools: Vec<Arc<dyn agent_core::types::AgentTool>> =
        ["bash", "read", "write", "edit", "grep", "find", "ls", "web_fetch", "web_search"]
            .iter()
            .filter_map(|name| {
                create_tool(
                    name,
                    Arc::clone(&backend) as Arc<dyn agent_core::tools::backend::ToolBackend>,
                )
            })
            .map(|t| -> Arc<dyn agent_core::types::AgentTool> {
                Arc::new(ToolAdapter::new(t, Arc::clone(&permission_ctx)))
            })
            .collect();

    tools.push(Arc::new(EnterPlanModeTool {
        permission_ctx: Arc::clone(&permission_ctx),
    }));
    tools.push(Arc::new(ExitPlanModeTool {
        permission_ctx: Arc::clone(&permission_ctx),
    }));
    tools.push(Arc::new(ToolAdapter::new(
        Box::new(SpawnSubagentTool),
        Arc::clone(&permission_ctx),
    )));

    tools
}

// ── MCP tool loading ─────────────────────────────────────────────────────────

/// Connect to each configured MCP server, discover its tools, and return them
/// wrapped as `types::AgentTool` for use with `Agent`.
///
/// Servers that fail to connect are skipped with a warning rather than aborting
/// the entire session — one broken MCP server should not block the agent.
async fn load_mcp_tools(
    servers: &[McpServerConfig],
    permission_ctx: Arc<Mutex<ToolPermissionContext>>,
) -> Vec<Arc<dyn agent_core::types::AgentTool>> {
    let mut tools: Vec<Arc<dyn agent_core::types::AgentTool>> = Vec::new();
    for config in servers {
        match McpClient::connect(config).await {
            Ok(client) => {
                let client = Arc::new(tokio::sync::Mutex::new(client));
                match discover_mcp_tools(&config.name, Arc::clone(&client)).await {
                    Ok(mcp_tools) => {
                        for t in mcp_tools {
                            tools.push(Arc::new(ToolAdapter::new(Box::new(t), Arc::clone(&permission_ctx))));
                        }
                    }
                    Err(e) => {
                        tracing::warn!("MCP server '{}' tool discovery failed: {e}", config.name);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("MCP server '{}' failed to connect: {e}", config.name);
            }
        }
    }
    tools
}

// ── Hook wiring ──────────────────────────────────────────────────────────────

/// Wire hooks from settings into `agent`'s tool lifecycle.
///
/// Reads the effective settings, creates a session-scoped `HookExecutor` with
/// the given `permission_mode`, attaches `BeforeToolCall`/`AfterToolCall` hooks,
/// and subscribes to `TurnEnd` to fire Stop hooks asynchronously.
///
/// Returns the `HookRunner` if hooks were configured, `None` otherwise.
fn wire_hooks(
    agent: &mut Agent,
    cwd: &std::path::Path,
    permission_mode: &str,
) -> Option<Arc<HookRunner>> {
    let agent_dir = get_agent_dir();
    let settings = SettingsManager::create(cwd, &agent_dir).get_effective_settings();
    let hooks_settings = settings.hooks?;

    let session_id = ulid::Ulid::new().to_string();
    let transcript_path = get_sessions_dir()
        .join(format!("{session_id}.jsonl"))
        .to_string_lossy()
        .into_owned();
    let executor = HookExecutor::new(session_id.clone(), cwd.to_string_lossy().to_string())
        .with_agent_type("coding-agent")
        .with_permission_mode(permission_mode)
        .with_transcript_path(transcript_path);
    let runner = Arc::new(HookRunner::new(executor, hooks_settings));
    let lifecycle = Arc::new(HooksLifecycle::new(Arc::clone(&runner)));
    agent.set_before_tool_call(lifecycle.clone());
    agent.set_after_tool_call(lifecycle);

    let stop_runner = Arc::clone(&runner);
    agent.subscribe(move |event| {
        use agent_core::AgentEvent;
        if let AgentEvent::TurnEnd { message, .. } = event {
            let last_msg = message.text();
            let r = Arc::clone(&stop_runner);
            tokio::spawn(async move {
                let _ = r.run_stop(Some(&last_msg), false).await;
            });
        }
    });

    Some(runner)
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Extract a short human-readable summary of tool args for display in the TUI.
fn args_to_preview(args: &serde_json::Value) -> String {
    for key in &["command", "file_path", "pattern", "path"] {
        if let Some(v) = args.get(key).and_then(|v| v.as_str()) {
            return v.chars().take(80).collect();
        }
    }
    args.to_string().chars().take(80).collect()
}

/// Run an agent session, sending [`AgentDelta`] events through `tx` instead of stdout.
///
/// Drops `tx` when the agent finishes so the receiver knows the stream ended.
pub async fn run_agent_session_to_channel(
    message: String,
    model_id: Option<String>,
    provider_id: Option<String>,
    api_key: Option<String>,
    tx: tokio::sync::mpsc::UnboundedSender<AgentDelta>,
    permission_mode: String,
) -> anyhow::Result<()> {
    if message.trim().is_empty() {
        return Ok(());
    }

    let model = build_model(provider_id.as_deref(), model_id.as_deref())?;
    let registry = Arc::new(ApiProviderRegistry::new());
    ai::register_builtin_into(&registry);

    let options = StreamOptions {
        api_key,
        ..StreamOptions::default()
    };
    let provider: Arc<dyn LlmProvider> = Arc::new(RegistryProvider {
        registry: Arc::clone(&registry),
        options,
    });

    let mut agent = Agent::new(AgentOptions::new(
        model,
        "You are a helpful coding assistant.",
        provider,
    ));

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let permission_ctx = build_permission_context(&permission_mode, &cwd);

    let backend = LocalBackend::new();
    let mut tools = create_default_tools(backend, Arc::clone(&permission_ctx));

    // Load MCP tools from settings and append to the tool list.
    let agent_dir = get_agent_dir();
    let settings = SettingsManager::create(&cwd, &agent_dir).get_effective_settings();
    if let Some(servers) = &settings.mcp_servers {
        tools.extend(load_mcp_tools(servers, Arc::clone(&permission_ctx)).await);
    }

    agent.set_tools(tools);

    let hook_runner = wire_hooks(&mut agent, &cwd, &permission_mode);

    // SessionStart hooks fire once before the agent starts its first turn.
    if let Some(runner) = &hook_runner {
        let _ = runner.run_session_start().await;
    }

    agent.subscribe(move |event| {
        use agent_core::AgentEvent;
        match event {
            AgentEvent::MessageUpdate { delta, .. } => {
                let _ = tx.send(AgentDelta::Text(delta.clone()));
            }
            AgentEvent::TurnEnd { message, .. } => {
                let is_fast = message.model.to_ascii_lowercase().contains("fast");
                let _ = tx.send(AgentDelta::TurnUsage {
                    usage: message.usage.clone(),
                    model: message.model.clone(),
                    is_fast,
                });
            }
            AgentEvent::ToolExecutionStart {
                tool_name, args, ..
            } => {
                let _ = tx.send(AgentDelta::ToolStart {
                    name: tool_name.clone(),
                    args_preview: args_to_preview(&args),
                });
            }
            AgentEvent::ToolExecutionEnd {
                tool_name,
                result,
                is_error,
                ..
            } => {
                use agent_core::types::Content;
                let text = result
                    .content
                    .iter()
                    .find_map(|c| {
                        if let Content::Text { text } = c {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .unwrap_or("");
                let first_nonempty = text
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .chars()
                    .take(100)
                    .collect::<String>();
                let _ = tx.send(AgentDelta::ToolEnd {
                    name: tool_name.clone(),
                    success: !is_error,
                    output_preview: first_nonempty,
                });
            }
            AgentEvent::RunError { error } => {
                let _ = tx.send(AgentDelta::Error(error.clone()));
            }
            _ => {}
        }
    });

    let run_result = agent.prompt_text(message).await.map_err(|e| anyhow::anyhow!(e));

    // SessionEnd hooks fire after the agent finishes (regardless of success).
    if let Some(runner) = hook_runner {
        let _ = runner.run_session_end().await;
    }

    run_result
}

/// Spawn a sub-agent as a team member, running it asynchronously in the background.
///
/// Resolves a model and provider using the same defaults as `run_agent_session`,
/// creates a `SpawnAgentConfig`, and delegates to `spawn_agent_in_team`.
/// Returns the spawned agent's unique ID.
///
/// This is the primary user-facing entry point for the sub-agent system,
/// wiring the session's LLM credentials into the team spawning path.
pub async fn spawn_subagent(
    prompt: String,
    agent_type: Option<String>,
    model_id: Option<String>,
    provider_id: Option<String>,
    api_key: Option<String>,
    team_name: Option<String>,
    cwd: Option<std::path::PathBuf>,
) -> anyhow::Result<String> {
    let model = build_model(provider_id.as_deref(), model_id.as_deref())?;

    let registry = Arc::new(ApiProviderRegistry::new());
    ai::register_builtin_into(&registry);

    let options = StreamOptions {
        api_key,
        ..StreamOptions::default()
    };
    let provider: Arc<dyn LlmProvider> = Arc::new(RegistryProvider {
        registry,
        options,
    });

    let effective_cwd = cwd
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
    // Sub-agents run inside a sandboxed VM; host-level enforcement is at the sandbox
    // boundary, so bypassPermissions is intentional here rather than inherited.
    let permission_ctx = build_permission_context("bypassPermissions", &effective_cwd);

    let backend = LocalBackend::new();
    let tools = create_default_tools(backend, permission_ctx);

    let name = agent_type.as_deref().unwrap_or("subagent").to_string();
    let config = SpawnAgentConfig {
        name,
        prompt,
        team_name,
        agent_type,
        model: None,
        cwd,
        provider,
        tools,
        parent_model: model,
    };

    // Passes empty existing_members — concurrent spawns may produce duplicate names
    // until callers thread live team membership into this call site.
    spawn_agent_in_team(config, &[])
        .await
        .map_err(|e: AgentError| anyhow::anyhow!("{e}"))
}

/// Run a single-shot agent session in print mode.
///
/// Resolves the model + provider, wires up tools, streams events to stdout,
/// and returns when the agent has finished.
pub async fn run_agent_session(
    message: String,
    model_id: Option<String>,
    provider_id: Option<String>,
    api_key: Option<String>,
    permission_mode: String,
) -> anyhow::Result<()> {
    if message.trim().is_empty() {
        return Ok(());
    }

    // 1. Build model descriptor.
    let model = build_model(provider_id.as_deref(), model_id.as_deref())?;

    // 2. Register built-in API providers and wrap in our adapter.
    let registry = Arc::new(ApiProviderRegistry::new());
    ai::register_builtin_into(&registry);

    let options = StreamOptions {
        api_key,
        ..StreamOptions::default()
    };

    let provider: Arc<dyn LlmProvider> = Arc::new(RegistryProvider {
        registry: Arc::clone(&registry),
        options,
    });

    // 3. Build agent.
    let mut agent = Agent::new(AgentOptions::new(
        model,
        "You are a helpful coding assistant.",
        provider,
    ));

    // 4. Build permission context from settings.json and attach default tools.
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let permission_ctx = build_permission_context(&permission_mode, &cwd);

    let backend = LocalBackend::new();
    let mut tools = create_default_tools(backend, Arc::clone(&permission_ctx));

    // 4b. Load MCP tools from settings and append to the tool list.
    let agent_dir = get_agent_dir();
    let settings = SettingsManager::create(&cwd, &agent_dir).get_effective_settings();
    if let Some(servers) = &settings.mcp_servers {
        tools.extend(load_mcp_tools(servers, Arc::clone(&permission_ctx)).await);
    }

    agent.set_tools(tools);

    // 4c. Wire hooks from settings into the agent tool lifecycle.
    let hook_runner = wire_hooks(&mut agent, &cwd, &permission_mode);

    // 4d. SessionStart hooks fire once before the agent starts its first turn.
    if let Some(runner) = &hook_runner {
        let _ = runner.run_session_start().await;
    }

    // 5. Subscribe to events: stream text deltas to stdout.
    let stdout = std::io::stdout();
    agent.subscribe(move |event| {
        use agent_core::AgentEvent;
        match event {
            AgentEvent::MessageUpdate { delta, .. } => {
                let mut out = stdout.lock();
                let _ = out.write_all(delta.as_bytes());
                let _ = out.flush();
            }
            AgentEvent::AgentEnd { .. } => {
                let mut out = stdout.lock();
                let _ = out.write_all(b"\n");
                let _ = out.flush();
            }
            AgentEvent::RunError { error } => {
                eprintln!("\nError: {error}");
            }
            AgentEvent::TurnEnd { message, .. } => {
                use agent_core::types::StopReason;
                if let Some(err) = &message.error_message {
                    eprintln!("Error: {err}");
                } else if message.stop_reason == StopReason::Error {
                    eprintln!("Error: agent stopped with error");
                }
            }
            _ => {}
        }
    });

    // 6. Send the message and wait.
    let run_result = agent.prompt_text(message).await.map_err(|e| anyhow::anyhow!(e));

    // 7. SessionEnd hooks fire after the agent finishes (regardless of success).
    if let Some(runner) = hook_runner {
        let _ = runner.run_session_end().await;
    }

    run_result
}
