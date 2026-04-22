//! Agent session wiring — connects the CLI to `agent-core` and the `ai` provider registry.
//!
//! `run_agent_session` is the concrete entry point for print-mode execution:
//! it resolves a model, creates an LLM provider, wires up the default tools,
//! subscribes to events, sends the user message, and waits for completion.

use std::io::Write as _;
use std::sync::{Arc, Mutex};

use crate::config::{CONFIG_DIR_NAME, get_agent_dir, get_sessions_dir};
use crate::core::agent::runner::AgentError;
use crate::core::hooks::HooksLifecycle;
use crate::core::hooks::executor::HookExecutor;
use crate::core::hooks::runner::HookRunner;
use crate::core::settings_manager::SettingsManager;
use crate::core::team::{SpawnAgentConfig, spawn_agent_in_team};
use crate::core::tools::plan_mode::{
    ENTER_PLAN_MODE_TOOL_NAME, EXIT_PLAN_MODE_TOOL_NAME, ExitPlanModeInput, PlanExitStrategy,
    enter_plan_mode, exit_plan_mode,
};
use crate::modes::interactive::approval::{ApprovalRequest, ApprovalResponse};
use crate::utils::permissions::engine::check_tool_permission;
use crate::utils::permissions::loader::{load_permissions_from_file, permissions_json_to_rules};
use crate::utils::permissions::parser::permission_rule_value_to_string;
use crate::utils::permissions::{
    PermissionBehavior, PermissionDecision, PermissionMode, PermissionRuleSource,
    ToolPermissionContext,
};
use agent_core::agent::{Agent, AgentOptions};
use agent_core::agent_loop::LlmProvider;
use agent_core::mcp::{McpClient, McpServerConfig};
use agent_core::tools::backend::LocalBackend;
use agent_core::tools::mcp_tool::discover_mcp_tools;
use agent_core::tools::{AgentTool as SimpleTool, ToolOutput, create_tool};
use agent_core::types::{AgentToolResult, Content, OnUpdateFn};
use ai::registry::{ApiProviderRegistry, StreamOptions};
use ai::types::{AssistantMessageEvent, InputType, Model, ModelCost, Usage};

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
/// How long the TUI approval dialog waits before treating silence as deny.
const APPROVAL_TIMEOUT_SECS: u64 = 300; // 5 minutes

/// `agent_core::types::AgentTool` trait (full interface expected by `Agent`).
///
/// Calls `PermissionEngine::check()` before each tool execution, returning a
/// denial message if the tool is blocked by settings.json permission rules.
struct ToolAdapter {
    inner: Box<dyn SimpleTool>,
    tool_description: String,
    tool_schema: serde_json::Value,
    permission_ctx: Arc<Mutex<ToolPermissionContext>>,
    approval_tx: Option<tokio::sync::mpsc::UnboundedSender<ApprovalRequest>>,
    /// Per-session rules set by AllowAlways/DenyAlways: key=tool_name, true=always allow.
    session_rules: Arc<Mutex<std::collections::HashMap<String, bool>>>,
}

impl ToolAdapter {
    fn new(
        tool: Box<dyn SimpleTool>,
        permission_ctx: Arc<Mutex<ToolPermissionContext>>,
        approval_tx: Option<tokio::sync::mpsc::UnboundedSender<ApprovalRequest>>,
        session_rules: Arc<Mutex<std::collections::HashMap<String, bool>>>,
    ) -> Self {
        let description = tool.description().to_string();
        let schema = tool.parameters_schema();
        Self {
            inner: tool,
            tool_description: description,
            tool_schema: schema,
            permission_ctx,
            approval_tx,
            session_rules,
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
        match decision {
            PermissionDecision::Deny { message, .. } => {
                return AgentToolResult {
                    content: vec![Content::Text {
                        text: format!("Permission denied: {message}"),
                    }],
                    details: serde_json::Value::Null,
                    is_error: true,
                };
            }
            PermissionDecision::Ask { message, .. } => {
                // Check session-level rules set by AllowAlways/DenyAlways first.
                let session_rule = {
                    let rules = self.session_rules.lock().unwrap();
                    rules.get(self.inner.name()).copied()
                };
                match session_rule {
                    Some(true) => {
                        // AllowAlways was set earlier — skip dialog.
                    }
                    Some(false) => {
                        return AgentToolResult {
                            content: vec![Content::Text {
                                text: format!(
                                    "The user denied permission to run '{}'. \
                                     Do not retry — ask the user how they would like to proceed.",
                                    self.inner.name()
                                ),
                            }],
                            details: serde_json::Value::Null,
                            is_error: true,
                        };
                    }
                    None => {
                        // No cached rule — ask the user via TUI.
                        if let Some(tx) = &self.approval_tx {
                            let (response_tx, response_rx) = tokio::sync::oneshot::channel();
                            let req = ApprovalRequest {
                                tool_name: self.inner.name().to_string(),
                                message: message.clone(),
                                response_tx,
                            };
                            if tx.send(req).is_ok() {
                                let result = tokio::time::timeout(
                                    std::time::Duration::from_secs(APPROVAL_TIMEOUT_SECS),
                                    response_rx,
                                )
                                .await;
                                match result {
                                    Ok(Ok(ApprovalResponse::Allow)) => {}
                                    Ok(Ok(ApprovalResponse::AllowAlways)) => {
                                        self.session_rules
                                            .lock()
                                            .unwrap()
                                            .insert(self.inner.name().to_string(), true);
                                    }
                                    Ok(Ok(ApprovalResponse::Deny)) => {
                                        return AgentToolResult {
                                            content: vec![Content::Text {
                                                text: format!(
                                                    "The user denied permission to run '{}'. \
                                                     Do not retry — ask the user how they would like to proceed.",
                                                    self.inner.name()
                                                ),
                                            }],
                                            details: serde_json::Value::Null,
                                            is_error: true,
                                        };
                                    }
                                    Ok(Ok(ApprovalResponse::DenyAlways)) => {
                                        self.session_rules
                                            .lock()
                                            .unwrap()
                                            .insert(self.inner.name().to_string(), false);
                                        return AgentToolResult {
                                            content: vec![Content::Text {
                                                text: format!(
                                                    "The user denied permission to run '{}'. \
                                                     Do not retry — ask the user how they would like to proceed.",
                                                    self.inner.name()
                                                ),
                                            }],
                                            details: serde_json::Value::Null,
                                            is_error: true,
                                        };
                                    }
                                    Ok(Err(_)) => {
                                        // TUI closed before responding (sender dropped) — abort silently.
                                        return AgentToolResult {
                                            content: vec![Content::Text {
                                                text: format!(
                                                    "Approval for '{}' could not be obtained (session closed). \
                                                     Stop and wait for further instructions.",
                                                    self.inner.name()
                                                ),
                                            }],
                                            details: serde_json::Value::Null,
                                            is_error: true,
                                        };
                                    }
                                    Err(_) => {
                                        // Timeout elapsed — user did not respond in time.
                                        return AgentToolResult {
                                            content: vec![Content::Text {
                                                text: format!(
                                                    "Approval for '{}' timed out ({}m{}s). \
                                                     Stop and wait for further instructions.",
                                                    self.inner.name(),
                                                    APPROVAL_TIMEOUT_SECS / 60,
                                                    APPROVAL_TIMEOUT_SECS % 60,
                                                ),
                                            }],
                                            details: serde_json::Value::Null,
                                            is_error: true,
                                        };
                                    }
                                }
                            } else {
                                return AgentToolResult {
                                    content: vec![Content::Text {
                                        text: format!(
                                            "Tool '{}' requires user approval but the approval channel is closed. \
                                             Stop and report this to the user.",
                                            self.inner.name()
                                        ),
                                    }],
                                    details: serde_json::Value::Null,
                                    is_error: true,
                                };
                            }
                        } else {
                            return AgentToolResult {
                                content: vec![Content::Text {
                                    text: format!(
                                        "Tool '{}' requires explicit user approval (ask rule matched). \
                                         This session cannot prompt for approval. Do not attempt alternative tools \
                                         or rephrased commands — stop and report to the user that approval is required.\n\
                                         Rule: {message}",
                                        self.inner.name()
                                    ),
                                }],
                                details: serde_json::Value::Null,
                                is_error: true,
                            };
                        }
                    }
                }
            }
            PermissionDecision::Allow { .. } => {}
        }

        let output: ToolOutput = self.inner.execute(args).await;
        // ToolOutput does not expose a details field; use Null as placeholder.
        AgentToolResult {
            content: output.content,
            details: serde_json::Value::Null,
            is_error: output.is_error,
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
                content: vec![Content::Text {
                    text: output.message,
                }],
                details: serde_json::Value::Null,
                is_error: false,
            },
            Err(e) => AgentToolResult {
                content: vec![Content::Text { text: e }],
                details: serde_json::Value::Null,
                is_error: true,
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
                content: vec![Content::Text {
                    text: output.message,
                }],
                details: serde_json::Value::Null,
                is_error: false,
            },
            Err(e) => AgentToolResult {
                content: vec![Content::Text { text: e }],
                details: serde_json::Value::Null,
                is_error: true,
            },
        }
    }
}

// ── Permission context builder ───────────────────────────────────────────────

/// Load permission rules from a settings file and insert them into `ctx`.
///
/// If the file does not exist or cannot be parsed, silently returns without
/// modifying `ctx`.  All rule behaviours (Allow / Deny / Ask) are supported.
fn load_rules_into(
    ctx: &mut ToolPermissionContext,
    path: &std::path::Path,
    source: PermissionRuleSource,
) {
    let Some(perms) = load_permissions_from_file(path) else {
        return;
    };
    for rule in permissions_json_to_rules(&perms, source) {
        let s = permission_rule_value_to_string(&rule.rule_value);
        match rule.rule_behavior {
            PermissionBehavior::Allow => ctx.add_allow_rule(rule.source, s),
            PermissionBehavior::Deny => ctx.add_deny_rule(rule.source, s),
            PermissionBehavior::Ask => ctx.add_ask_rule(rule.source, s),
        }
    }
}

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
    load_rules_into(
        &mut ctx,
        &global_settings_path,
        PermissionRuleSource::UserSettings,
    );

    let project_settings_path = cwd.join(CONFIG_DIR_NAME).join("settings.json");
    load_rules_into(
        &mut ctx,
        &project_settings_path,
        PermissionRuleSource::ProjectSettings,
    );

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

struct SpawnSubagentTool {
    provider_id: Option<String>,
    api_key: Option<String>,
    permission_ctx: Arc<Mutex<ToolPermissionContext>>,
}

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

        let model_id = args
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let cwd = args
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from);

        // Always read the current permission mode from the live context at spawn
        // time so the sub-agent reflects whatever mode the parent is in right
        // now (e.g. if the parent has entered plan mode, the sub-agent must
        // also run in plan mode — it must not be more permissive).
        let (is_plan_mode, effective_mode) = {
            let ctx = self.permission_ctx.lock().unwrap();
            (ctx.mode == PermissionMode::Plan, ctx.mode.to_string())
        };

        // N4: Refuse to spawn a sub-agent while the parent is in Plan mode.
        // Plan mode is a read-only exploration phase; spawning sub-agents that
        // could write files would silently bypass the plan-mode contract.
        if is_plan_mode {
            return ToolOutput {
                content: vec![Content::Text {
                    text: "Cannot spawn sub-agent while in Plan mode. \
                           Exit Plan mode first (ExitPlanMode tool), then retry."
                        .to_string(),
                }],
                is_error: true,
            };
        }

        match spawn_subagent(
            task,
            None,
            model_id,
            self.provider_id.clone(),
            self.api_key.clone(),
            None,
            cwd,
            effective_mode,
        )
        .await
        {
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
/// rules from settings.json before each execution.  `SpawnSubagent` is always
/// registered.  `EnterPlanMode` and `ExitPlanMode` are only registered for
/// top-level agents — sub-agents must NOT enter plan mode because they run
/// inside an isolated `ToolPermissionContext` that the parent cannot observe,
/// which would silently bypass the parent's permission model.
fn create_default_tools(
    backend: Arc<LocalBackend>,
    permission_ctx: Arc<Mutex<ToolPermissionContext>>,
    is_subagent: bool,
    provider_id: Option<String>,
    api_key: Option<String>,
    approval_tx: Option<tokio::sync::mpsc::UnboundedSender<ApprovalRequest>>,
    session_rules: Arc<Mutex<std::collections::HashMap<String, bool>>>,
) -> Vec<Arc<dyn agent_core::types::AgentTool>> {
    let mut tools: Vec<Arc<dyn agent_core::types::AgentTool>> = [
        "bash",
        "read",
        "write",
        "edit",
        "grep",
        "find",
        "ls",
        "web_fetch",
        "web_search",
    ]
    .iter()
    .filter_map(|name| {
        create_tool(
            name,
            Arc::clone(&backend) as Arc<dyn agent_core::tools::backend::ToolBackend>,
        )
    })
    .map(|t| -> Arc<dyn agent_core::types::AgentTool> {
        Arc::new(ToolAdapter::new(
            t,
            Arc::clone(&permission_ctx),
            approval_tx.clone(),
            Arc::clone(&session_rules),
        ))
    })
    .collect();

    if !is_subagent {
        tools.push(Arc::new(EnterPlanModeTool {
            permission_ctx: Arc::clone(&permission_ctx),
        }));
        tools.push(Arc::new(ExitPlanModeTool {
            permission_ctx: Arc::clone(&permission_ctx),
        }));
    }
    tools.push(Arc::new(ToolAdapter::new(
        Box::new(SpawnSubagentTool {
            provider_id,
            api_key,
            permission_ctx: Arc::clone(&permission_ctx),
        }),
        Arc::clone(&permission_ctx),
        None,
        Arc::new(Mutex::new(std::collections::HashMap::new())),
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
                            tools.push(Arc::new(ToolAdapter::new(
                                Box::new(t),
                                Arc::clone(&permission_ctx),
                                None,
                                Arc::new(Mutex::new(std::collections::HashMap::new())),
                            )));
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
                match r.run_stop(Some(&last_msg), false).await {
                    Ok(_result) => {
                        // Stop hook ran successfully.
                        // Note: blocking outcome is not yet supported in this architecture —
                        // the agent loop cannot be paused from a TurnEnd callback (fire-and-forget).
                        // Configure stop behavior via session-level controls instead.
                        // TODO: surface blocking outcome once a synchronous stop hook path exists.
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Stop hook failed");
                    }
                }
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
    approval_tx: Option<tokio::sync::mpsc::UnboundedSender<ApprovalRequest>>,
    session_rules: Arc<Mutex<std::collections::HashMap<String, bool>>>,
) -> anyhow::Result<()> {
    if message.trim().is_empty() {
        return Ok(());
    }

    let model = build_model(provider_id.as_deref(), model_id.as_deref())?;
    let registry = Arc::new(ApiProviderRegistry::new());
    ai::register_builtin_into(&registry);

    // Clone before move into StreamOptions so we can pass to SpawnSubagentTool.
    let provider_id_for_subagent = provider_id.clone();
    let api_key_for_subagent = api_key.clone();

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
    let mut tools = create_default_tools(
        backend,
        Arc::clone(&permission_ctx),
        false,
        provider_id_for_subagent,
        api_key_for_subagent,
        approval_tx,
        session_rules,
    );

    // Load MCP tools from settings and append to the tool list.
    let agent_dir = get_agent_dir();
    let settings = SettingsManager::create(&cwd, &agent_dir).get_effective_settings();
    if let Some(servers) = &settings.mcp_servers {
        tools.extend(load_mcp_tools(servers, Arc::clone(&permission_ctx)).await);
    }

    agent.set_tools(tools);

    let hook_runner = wire_hooks(&mut agent, &cwd, &permission_mode);

    // SessionStart hooks fire once before the agent starts its first turn.
    if let Some(runner) = &hook_runner
        && let Err(e) = runner.run_session_start().await
    {
        tracing::warn!(error = %e, "SessionStart hook failed — continuing session");
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

    let run_result = agent
        .prompt_text(message)
        .await
        .map_err(|e| anyhow::anyhow!(e));

    // SessionEnd hooks fire after the agent finishes (regardless of success).
    if let Some(runner) = hook_runner
        && let Err(e) = runner.run_session_end().await
    {
        tracing::warn!(error = %e, "SessionEnd hook failed");
    }

    run_result
}

// ── Approval-path unit tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};

    use agent_core::types::AgentTool as _;

    use crate::modes::interactive::approval::ApprovalResponse;
    use crate::utils::permissions::{PermissionRuleSource, ToolPermissionContext};

    // ── Mock tool ────────────────────────────────────────────────────────────

    struct MockTool {
        tool_name: &'static str,
        executed: Arc<AtomicBool>,
    }

    impl MockTool {
        fn new(name: &'static str) -> (Self, Arc<AtomicBool>) {
            let flag = Arc::new(AtomicBool::new(false));
            (
                Self {
                    tool_name: name,
                    executed: flag.clone(),
                },
                flag,
            )
        }
    }

    #[async_trait::async_trait]
    impl SimpleTool for MockTool {
        fn name(&self) -> &str {
            self.tool_name
        }
        fn description(&self) -> &str {
            "mock"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(&self, _args: serde_json::Value) -> agent_core::tools::ToolOutput {
            self.executed.store(true, Ordering::Relaxed);
            agent_core::tools::ToolOutput {
                content: vec![Content::Text {
                    text: "ok".to_string(),
                }],
                is_error: false,
            }
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn ask_ctx(tool_name: &str) -> Arc<Mutex<ToolPermissionContext>> {
        let mut ctx = ToolPermissionContext::default();
        ctx.add_ask_rule(PermissionRuleSource::Session, tool_name.to_owned());
        Arc::new(Mutex::new(ctx))
    }

    fn first_text(result: &AgentToolResult) -> &str {
        result
            .content
            .iter()
            .find_map(|c| {
                if let Content::Text { text } = c {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .unwrap_or("")
    }

    fn make_adapter(
        name: &'static str,
    ) -> (
        ToolAdapter,
        Arc<AtomicBool>,
        tokio::sync::mpsc::UnboundedReceiver<ApprovalRequest>,
    ) {
        let (tool, executed) = MockTool::new(name);
        let ctx = ask_ctx(name);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let session_rules = Arc::new(Mutex::new(HashMap::<String, bool>::new()));
        let adapter = ToolAdapter::new(Box::new(tool), ctx, Some(tx), session_rules);
        (adapter, executed, rx)
    }

    // ── Tests ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn approval_allow_executes_tool() {
        let (adapter, executed, mut rx) = make_adapter("test_tool");

        tokio::spawn(async move {
            if let Some(req) = rx.recv().await {
                let _ = req.response_tx.send(ApprovalResponse::Allow);
            }
        });

        let result = adapter.execute("id", serde_json::json!({}), None, None).await;
        assert!(!result.is_error, "Allow should not be an error");
        assert!(executed.load(Ordering::Relaxed), "Allow should execute the tool");
    }

    #[tokio::test]
    async fn approval_deny_blocks_tool() {
        let (adapter, executed, mut rx) = make_adapter("test_tool");

        tokio::spawn(async move {
            if let Some(req) = rx.recv().await {
                let _ = req.response_tx.send(ApprovalResponse::Deny);
            }
        });

        let result = adapter.execute("id", serde_json::json!({}), None, None).await;
        assert!(result.is_error, "Deny should be an error");
        assert!(!executed.load(Ordering::Relaxed), "Deny should not execute the tool");
        assert!(
            first_text(&result).contains("denied"),
            "Deny message should contain 'denied': {}",
            first_text(&result)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn approval_timeout_returns_error() {
        let (adapter, executed, mut rx) = make_adapter("test_tool");

        // Receive the request but never respond — hold response_tx alive until
        // after the approval timeout fires.
        tokio::spawn(async move {
            let req = rx.recv().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(
                APPROVAL_TIMEOUT_SECS + 60,
            ))
            .await;
            drop(req);
        });

        let result = adapter.execute("id", serde_json::json!({}), None, None).await;
        assert!(result.is_error, "Timeout should be an error");
        assert!(
            !executed.load(Ordering::Relaxed),
            "Timeout should not execute the tool"
        );
        assert!(
            first_text(&result).contains("timed out"),
            "Timeout message should contain 'timed out': {}",
            first_text(&result)
        );
    }

    #[tokio::test]
    async fn approval_session_closed_returns_error() {
        let (adapter, executed, mut rx) = make_adapter("test_tool");

        // Simulate TUI close: receive the request and drop the response sender.
        tokio::spawn(async move {
            if let Some(req) = rx.recv().await {
                drop(req.response_tx);
            }
        });

        let result = adapter.execute("id", serde_json::json!({}), None, None).await;
        assert!(result.is_error, "Session closed should be an error");
        assert!(
            !executed.load(Ordering::Relaxed),
            "Session closed should not execute the tool"
        );
        assert!(
            first_text(&result).contains("session closed"),
            "Session closed message should contain 'session closed': {}",
            first_text(&result)
        );
    }
}

/// Spawn a sub-agent as a team member, running it asynchronously in the background.
///
/// Resolves a model and provider using the same defaults as `run_agent_session`,
/// creates a `SpawnAgentConfig`, and delegates to `spawn_agent_in_team`.
/// Returns the spawned agent's unique ID.
///
/// This is the primary user-facing entry point for the sub-agent system,
/// wiring the session's LLM credentials into the team spawning path.
#[allow(clippy::too_many_arguments)]
pub async fn spawn_subagent(
    prompt: String,
    agent_type: Option<String>,
    model_id: Option<String>,
    provider_id: Option<String>,
    api_key: Option<String>,
    team_name: Option<String>,
    cwd: Option<std::path::PathBuf>,
    permission_mode: String,
) -> anyhow::Result<String> {
    let model = build_model(provider_id.as_deref(), model_id.as_deref())?;

    let registry = Arc::new(ApiProviderRegistry::new());
    ai::register_builtin_into(&registry);

    // Clone before move into StreamOptions so SpawnSubagentTool can carry them.
    let provider_id_for_subagent = provider_id.clone();
    let api_key_for_subagent = api_key.clone();

    let options = StreamOptions {
        api_key,
        ..StreamOptions::default()
    };
    let provider: Arc<dyn LlmProvider> = Arc::new(RegistryProvider { registry, options });

    let effective_cwd = cwd.clone().unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    });
    let permission_ctx = build_permission_context(&permission_mode, &effective_cwd);

    let backend = LocalBackend::new();
    let tools = create_default_tools(
        backend,
        permission_ctx,
        true,
        provider_id_for_subagent,
        api_key_for_subagent,
        None,
        Arc::new(Mutex::new(std::collections::HashMap::new())),
    );

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

    // Clone before move into StreamOptions so SpawnSubagentTool can carry them.
    let provider_id_for_subagent = provider_id.clone();
    let api_key_for_subagent = api_key.clone();

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
    let mut tools = create_default_tools(
        backend,
        Arc::clone(&permission_ctx),
        false,
        provider_id_for_subagent,
        api_key_for_subagent,
        None,
        Arc::new(Mutex::new(std::collections::HashMap::new())),
    );

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
    if let Some(runner) = &hook_runner
        && let Err(e) = runner.run_session_start().await
    {
        tracing::warn!(error = %e, "SessionStart hook failed — continuing session");
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
    let run_result = agent
        .prompt_text(message)
        .await
        .map_err(|e| anyhow::anyhow!(e));

    // 7. SessionEnd hooks fire after the agent finishes (regardless of success).
    if let Some(runner) = hook_runner
        && let Err(e) = runner.run_session_end().await
    {
        tracing::warn!(error = %e, "SessionEnd hook failed");
    }

    run_result
}
