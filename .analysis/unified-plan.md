# Sub-Agent 系统 Rust 移植实现方案

## 执行概述

本方案覆盖 CC sub-agent 系统的完整 Rust 实现，分为 3 个模块和 12 个阶段，目标交付生产就绪的 sub-agent 支持，支持 fork、team 和 spawn 等所有模式。

**预期工作量**：1-2 周（12 个并行可做的子任务）  
**依赖**：Sage v0.2.0 基础框架（CLI、Agent 核心、权限系统）  
**交付物**：
- `sagec crate::agent` - 核心 agent 执行
- `sagec crate::subagent` - 上下文隔离和 fork
- `sagec crate::team` - 多 agent 管理
- 单元测试和集成测试
- 迁移指南文档

---

## 模块 1：Agent 定义和加载

### 目标
支持 agent 定义的声明、验证和动态加载。

### Phase 1.1：AgentDef Struct 和序列化

**文件**：`sagec/src/agent/definition.rs`

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Agent 定义 - 对应 CC 的 AgentDefinition
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentDef {
    pub agent_type: String,           // 唯一标识
    pub when_to_use: String,          // 使用说明
    pub tools: Vec<String>,           // 工具列表（支持 ["*"] 全部继承）
    pub max_turns: Option<u32>,       // 最大轮数
    pub model: Option<AgentModel>,    // 模型（Opus/Sonnet/Haiku/inherit）
    pub permission_mode: Option<PermissionMode>,
    pub source: AgentSource,          // built-in | custom | plugin
    pub mcp_servers: Option<Vec<MCPServerSpec>>,
    pub system_prompt_fn: String,     // 返回 SystemPrompt 的 fn 路径
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AgentModel {
    Opus,
    Sonnet,
    Haiku,
    Inherit,  // 继承父 agent 的模型
    Custom(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AgentSource {
    BuiltIn,
    Custom,
    Plugin,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MCPServerSpec {
    Reference(String),  // 引用已有服务名
    Inline(HashMap<String, serde_json::Value>),  // 内联配置
}

/// Agent 定义加载器 - 扫描 built-in 和自定义 agents
pub struct AgentLoader {
    agents: HashMap<String, AgentDef>,
}

impl AgentLoader {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }

    /// 加载内置 agents
    pub fn load_builtin(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // 加载 6 个内置 agents：
        // - general_purpose（通用目的 agent）
        // - plan（规划 agent）
        // - explore（探索 agent）
        // - verification（验证 agent）
        // - code_guide（编码指南 agent）
        // - fork（隐式 fork agent）
        Ok(())
    }

    /// 加载自定义 agents（从目录或配置文件）
    pub fn load_custom<P: AsRef<std::path::Path>>(
        &mut self,
        dir: P,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // 扫描 YAML/JSON 文件，解析为 AgentDef
        Ok(())
    }

    pub fn get(&self, agent_type: &str) -> Option<&AgentDef> {
        self.agents.get(agent_type)
    }

    pub fn list_all(&self) -> Vec<&AgentDef> {
        self.agents.values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_def_serialization() {
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
    }
}
```

### Phase 1.2：内置 Agent 注册

**文件**：`sagec/src/agent/builtin.rs`

```rust
use super::definition::{AgentDef, AgentModel, AgentSource};
use std::collections::HashMap;

pub fn register_builtin_agents() -> HashMap<String, AgentDef> {
    let mut agents = HashMap::new();

    // General Purpose Agent
    agents.insert(
        "general-purpose".to_string(),
        AgentDef {
            agent_type: "general-purpose".to_string(),
            when_to_use: "General-purpose research and coding tasks".to_string(),
            tools: vec!["*".to_string()],
            max_turns: Some(200),
            model: Some(AgentModel::Opus),
            permission_mode: None,
            source: AgentSource::BuiltIn,
            mcp_servers: None,
            system_prompt_fn: "get_general_purpose_prompt".to_string(),
        },
    );

    // Plan Agent
    agents.insert(
        "plan".to_string(),
        AgentDef {
            agent_type: "plan".to_string(),
            when_to_use: "Implementation planning and design".to_string(),
            tools: vec!["read".to_string(), "grep".to_string(), "glob".to_string()],
            max_turns: Some(50),
            model: Some(AgentModel::Sonnet),
            permission_mode: None,
            source: AgentSource::BuiltIn,
            mcp_servers: None,
            system_prompt_fn: "get_plan_prompt".to_string(),
        },
    );

    // Explore Agent
    agents.insert(
        "explore".to_string(),
        AgentDef {
            agent_type: "explore".to_string(),
            when_to_use: "Fast codebase exploration".to_string(),
            tools: vec!["glob".to_string(), "grep".to_string(), "read".to_string()],
            max_turns: Some(30),
            model: Some(AgentModel::Haiku),
            permission_mode: None,
            source: AgentSource::BuiltIn,
            mcp_servers: None,
            system_prompt_fn: "get_explore_prompt".to_string(),
        },
    );

    // Fork Agent (implicit)
    agents.insert(
        "fork".to_string(),
        AgentDef {
            agent_type: "fork".to_string(),
            when_to_use: "Implicit fork - inherits full conversation context".to_string(),
            tools: vec!["*".to_string()],
            max_turns: Some(200),
            model: Some(AgentModel::Inherit),
            permission_mode: Some(PermissionMode::Bubble),
            source: AgentSource::BuiltIn,
            mcp_servers: None,
            system_prompt_fn: "get_fork_prompt".to_string(),
        },
    );

    agents
}
```

---

## 模块 2：上下文隔离和 Subagent 管理

### 目标
实现上下文克隆、隔离和参数传递。

### Phase 2.1：ToolContext 和隔离接口

**文件**：`sagec/src/context.rs`（扩展）

```rust
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Agent 执行工具上下文 - 对应 CC 的 ToolUseContext
#[derive(Clone)]
pub struct ToolContext {
    pub agent_id: AgentId,
    pub options: ContextOptions,
    pub tools: Arc<Tools>,  // 共享 Tools 列表
    
    // 隔离状态（每个 agent 独占）
    pub read_file_state: Arc<RwLock<FileStateCache>>,
    pub content_replacement_state: Arc<RwLock<ContentReplacementState>>,
    
    // Abort 控制（可共享或隔离）
    pub abort_controller: Arc<tokio_util::sync::CancellationToken>,
    
    // 权限和隔离
    pub permission_context: PermissionContext,
    pub should_avoid_permission_prompts: bool,
    
    // 回调和钩子
    pub app_state_setter: Arc<dyn Fn(AppState) -> BoxFuture<'static, ()> + Send + Sync>,
    pub on_query_progress: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// 缓存关键参数 - 确保 prompt cache 一致性
#[derive(Clone, Debug)]
pub struct CacheSafeParams {
    pub system_prompt: SystemPrompt,
    pub user_context: HashMap<String, String>,
    pub system_context: HashMap<String, String>,
    pub tool_use_context: ToolContext,
    pub fork_context_messages: Vec<Message>,
}

impl ToolContext {
    /// 创建子 agent 上下文（完全隔离）
    pub fn create_subagent_context(
        &self,
        overrides: Option<SubagentContextOverrides>,
    ) -> Self {
        let overrides = overrides.unwrap_or_default();
        
        let abort_controller = if let Some(ac) = overrides.abort_controller {
            Arc::new(ac)
        } else if overrides.share_abort_controller {
            Arc::clone(&self.abort_controller)
        } else {
            // 创建子 token
            Arc::new(tokio_util::sync::CancellationToken::new())
        };

        let should_avoid_prompts = if overrides.share_abort_controller {
            self.should_avoid_permission_prompts
        } else {
            true  // 异步子 agent 避免权限提示
        };

        ToolContext {
            agent_id: AgentId::new(),  // 新 ID
            options: overrides.options.unwrap_or_else(|| self.options.clone()),
            tools: Arc::clone(&self.tools),  // 共享工具列表
            read_file_state: Arc::new(RwLock::new(
                self.read_file_state.blocking_read().clone()
            )),
            content_replacement_state: Arc::new(RwLock::new(
                overrides.content_replacement_state
                    .unwrap_or_else(|| {
                        self.content_replacement_state.blocking_read().clone()
                    })
            )),
            abort_controller,
            permission_context: self.permission_context.clone(),
            should_avoid_permission_prompts: should_avoid_prompts,
            app_state_setter: if overrides.share_set_app_state {
                Arc::clone(&self.app_state_setter)
            } else {
                Arc::new(|_| Box::pin(async {}))  // no-op
            },
            on_query_progress: overrides.on_query_progress
                .or_else(|| self.on_query_progress.as_ref().map(Arc::clone)),
        }
    }
}

#[derive(Clone, Default)]
pub struct SubagentContextOverrides {
    pub abort_controller: Option<tokio_util::sync::CancellationToken>,
    pub share_abort_controller: bool,
    pub share_set_app_state: bool,
    pub options: Option<ContextOptions>,
    pub content_replacement_state: Option<ContentReplacementState>,
    pub on_query_progress: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_subagent_context_isolation() {
        let parent = create_test_context();
        let child = parent.create_subagent_context(None);
        
        assert_ne!(parent.agent_id, child.agent_id);
        // 文件状态应该是克隆的，不是共享的
        assert!(!Arc::ptr_eq(&parent.read_file_state, &child.read_file_state));
    }
}
```

### Phase 2.2：ForkedAgent 和 CacheSafeParams

**文件**：`sagec/src/agent/forked.rs`

```rust
use crate::context::{CacheSafeParams, ToolContext};
use crate::message::Message;
use futures::stream::BoxStream;

/// Fork 消息处理 - 替换 tool_result 为占位符
pub const FORK_PLACEHOLDER_RESULT: &str = "Fork started — processing in background";
pub const FORK_BOILERPLATE_TAG: &str = "fork_boilerplate_tag";

pub fn build_forked_messages(
    parent_messages: &[Message],
) -> Vec<Message> {
    parent_messages
        .iter()
        .map(|msg| {
            if let Message::User {
                ref content,
                ..
            } = msg
            {
                // 检查 fork boilerplate tag
                if let ContentBlock::Text(text) = content {
                    if text.contains(&format!("<{}>", FORK_BOILERPLATE_TAG)) {
                        return msg.clone();  // 已经是 fork child
                    }
                }
            }

            if let Message::Assistant { ref content, .. } = msg {
                // 替换所有 tool_result 块
                let mut new_content = Vec::new();
                for block in content {
                    match block {
                        ContentBlock::ToolResult { tool_use_id, .. } => {
                            new_content.push(ContentBlock::ToolResult {
                                tool_use_id: tool_use_id.clone(),
                                content: FORK_PLACEHOLDER_RESULT.to_string(),
                            });
                        }
                        _ => new_content.push(block.clone()),
                    }
                }
                Message::Assistant {
                    content: new_content,
                    ..msg.clone()
                }
            } else {
                msg.clone()
            }
        })
        .collect()
}

pub fn is_in_fork_child(messages: &[Message]) -> bool {
    messages.iter().any(|m| {
        if let Message::User { ref content, .. } = m {
            if let ContentBlock::Text(text) = content {
                text.contains(&format!("<{}>", FORK_BOILERPLATE_TAG))
            } else {
                false
            }
        } else {
            false
        }
    })
}

/// 运行 fork agent 并收集结果
pub async fn run_forked_agent(
    prompt_messages: Vec<Message>,
    cache_safe_params: CacheSafeParams,
    can_use_tool: Box<dyn CanUseTool>,
    query_source: QuerySource,
) -> Result<(Vec<Message>, UsageStats), AgentError> {
    let CacheSafeParams {
        system_prompt,
        user_context,
        system_context,
        tool_use_context,
        fork_context_messages,
    } = cache_safe_params;

    let isolated_context = tool_use_context.create_subagent_context(None);

    let mut all_messages = fork_context_messages.clone();
    all_messages.extend(prompt_messages);

    // 执行 query 循环，采用隔离上下文
    let (result_messages, usage) = run_agent(
        RunAgentParams {
            system_prompt,
            user_context,
            system_context,
            tool_context: isolated_context,
            can_use_tool,
            query_source,
            messages: all_messages,
            max_turns: None,
        },
    ).await?;

    Ok((result_messages, usage))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fork_message_processing() {
        let messages = vec![
            Message::Assistant {
                content: vec![
                    ContentBlock::ToolUse { .. },
                    ContentBlock::ToolResult {
                        tool_use_id: "123".to_string(),
                        content: "result data".to_string(),
                    },
                ],
                ..Default::default()
            },
        ];

        let forked = build_forked_messages(&messages);
        assert_eq!(forked.len(), 1);
        
        if let Message::Assistant { content, .. } = &forked[0] {
            assert!(content.iter().any(|b| {
                matches!(b, ContentBlock::ToolResult { content, .. } 
                    if content == FORK_PLACEHOLDER_RESULT)
            }));
        }
    }

    #[test]
    fn test_fork_child_detection() {
        let messages = vec![
            Message::User {
                content: ContentBlock::Text(format!("<{}>", FORK_BOILERPLATE_TAG)),
                ..Default::default()
            },
        ];

        assert!(is_in_fork_child(&messages));
    }
}
```

---

## 模块 3：Agent 执行引擎

### 目标
实现 agent 运行、工具执行和权限检查的核心循环。

### Phase 3.1：RunAgent 异步流

**文件**：`sagec/src/agent/runner.rs`

```rust
use futures::stream::Stream;
use pin_project::pin_project;

pub struct RunAgentParams {
    pub agent_def: AgentDef,
    pub system_prompt: SystemPrompt,
    pub user_context: HashMap<String, String>,
    pub system_context: HashMap<String, String>,
    pub tool_context: ToolContext,
    pub can_use_tool: Box<dyn CanUseTool>,
    pub query_source: QuerySource,
    pub messages: Vec<Message>,
    pub max_turns: Option<u32>,
    pub allowed_tools: Option<Vec<String>>,
    pub model_override: Option<AgentModel>,
    pub on_cache_safe_params: Option<Box<dyn Fn(CacheSafeParams) + Send>>,
}

/// RunAgent 返回消息流
pub async fn run_agent(
    params: RunAgentParams,
) -> impl Stream<Item = Result<Message, AgentError>> {
    // 1. 初始化 MCP 服务
    let mcp_services = initialize_mcp_servers(&params.agent_def).await?;
    
    // 2. 构建 API 请求参数
    let resolved_model = resolve_model(&params);
    let tools = resolve_tools(&params);
    
    // 3. 调用 onCacheSafeParams 回调
    if let Some(callback) = params.on_cache_safe_params {
        let cache_params = CacheSafeParams {
            system_prompt: params.system_prompt.clone(),
            user_context: params.user_context.clone(),
            system_context: params.system_context.clone(),
            tool_use_context: params.tool_context.clone(),
            fork_context_messages: params.messages.clone(),
        };
        callback(cache_params);
    }

    // 4. 执行 query 循环
    run_query_loop(QueryLoopParams {
        model: resolved_model,
        system_prompt: params.system_prompt,
        tools,
        messages: params.messages,
        max_turns: params.max_turns.unwrap_or(200),
        tool_context: params.tool_context,
        can_use_tool: params.can_use_tool,
        allowed_tools: params.allowed_tools,
        query_source: params.query_source,
    }).await
}

#[pin_project]
pub struct RunAgentStream {
    #[pin]
    inner: Box<dyn Stream<Item = Result<Message, AgentError>> + Send>,
}

impl Stream for RunAgentStream {
    type Item = Result<Message, AgentError>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.project().inner.poll_next(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_run_agent_basic() {
        let params = create_test_run_agent_params();
        let stream = run_agent(params).await;
        
        let messages: Vec<_> = stream.collect().await;
        assert!(!messages.is_empty());
    }
}
```

### Phase 3.2：Query 循环和工具执行

**文件**：`sagec/src/agent/query_loop.rs`

```rust
pub struct QueryLoopParams {
    pub model: AgentModel,
    pub system_prompt: SystemPrompt,
    pub tools: Vec<Tool>,
    pub messages: Vec<Message>,
    pub max_turns: u32,
    pub tool_context: ToolContext,
    pub can_use_tool: Box<dyn CanUseTool>,
    pub allowed_tools: Option<Vec<String>>,
    pub query_source: QuerySource,
}

pub async fn run_query_loop(
    params: QueryLoopParams,
) -> impl Stream<Item = Result<Message, AgentError>> {
    async_stream::stream! {
        let mut messages = params.messages;
        let mut turn = 0;

        loop {
            if turn >= params.max_turns {
                break;
            }

            // 1. 调用 API（Claude API）
            let response = call_claude_api(ClaudeRequest {
                model: params.model.clone(),
                system_prompt: params.system_prompt.clone(),
                tools: params.tools.clone(),
                messages: messages.clone(),
            }).await?;

            let assistant_msg = Message::Assistant {
                content: response.content,
                usage: response.usage,
                ..Default::default()
            };

            yield Ok(assistant_msg.clone());
            messages.push(assistant_msg.clone());

            // 2. 检查工具使用
            if !response.content.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. })) {
                // 无工具使用，流程结束
                break;
            }

            // 3. 执行工具
            for block in &response.content {
                if let ContentBlock::ToolUse { tool_name, input, id } = block {
                    // 权限检查
                    if let Err(e) = params.can_use_tool(
                        tool_name,
                        params.allowed_tools.as_ref(),
                    ) {
                        yield Ok(Message::ToolResult {
                            tool_use_id: id.clone(),
                            content: format!("Permission denied: {}", e),
                        });
                        continue;
                    }

                    // 执行工具
                    let tool_result = execute_tool(tool_name, input).await?;
                    
                    yield Ok(Message::ToolResult {
                        tool_use_id: id.clone(),
                        content: tool_result,
                    });
                    
                    messages.push(Message::ToolResult {
                        tool_use_id: id.clone(),
                        content: tool_result,
                    });
                }
            }

            turn += 1;
        }
    }
}

async fn call_claude_api(
    request: ClaudeRequest,
) -> Result<ClaudeResponse, AgentError> {
    // 调用 Claude API（需要 anthropic SDK）
    // 这是框架的职责，返回完整响应
    todo!()
}

async fn execute_tool(
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<String, AgentError> {
    // 根据工具名称调用相应的工具
    match tool_name {
        "read" => execute_read_tool(input).await,
        "glob" => execute_glob_tool(input).await,
        "grep" => execute_grep_tool(input).await,
        _ => Err(AgentError::ToolNotFound(tool_name.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_query_loop_no_tool_use() {
        // 测试无工具使用的情况
        let stream = run_query_loop(create_test_query_params()).await;
        let messages: Vec<_> = stream.collect().await;
        assert!(!messages.is_empty());
    }

    #[tokio::test]
    async fn test_tool_permission_check() {
        // 测试权限检查
        let params = QueryLoopParams {
            allowed_tools: Some(vec!["read".to_string()]),
            ..create_test_query_params()
        };
        // 验证被禁止的工具被拒绝
    }
}
```

---

## 模块 4：团队和并行 Agent 管理（可选）

### Phase 4.1：Team 类型和 Agent 生成

**文件**：`sagec/src/team/mod.rs`

```rust
pub struct TeamConfig {
    pub name: String,
    pub members: Vec<TeamMember>,
}

pub struct TeamMember {
    pub agent_id: AgentId,
    pub agent_type: String,
    pub name: String,
    pub status: MemberStatus,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MemberStatus {
    Idle,
    Running,
    Done,
    Failed,
}

pub struct SpawnAgentConfig {
    pub name: String,
    pub prompt: String,
    pub team_name: Option<String>,
    pub agent_type: Option<String>,
    pub model: Option<AgentModel>,
    pub cwd: Option<std::path::PathBuf>,
}

pub async fn spawn_agent_in_team(
    config: SpawnAgentConfig,
    parent_context: &ToolContext,
) -> Result<AgentId, AgentError> {
    // 1. 生成唯一名称
    let unique_name = generate_unique_name(&config.name, config.team_name.as_deref()).await?;
    
    // 2. 创建 team member 记录
    let agent_id = AgentId::new();
    
    // 3. 启动 agent（可以是同进程或异步）
    let agent_def = match config.agent_type {
        Some(ref agent_type) => load_agent_def(agent_type)?,
        None => load_agent_def("general-purpose")?,  // 默认使用通用 agent
    };

    // 4. 继承模型
    let model = match config.model {
        Some(AgentModel::Inherit) | None => parent_context.options.model.clone(),
        Some(m) => m,
    };

    // 5. 在后台运行 agent
    tokio::spawn({
        let agent_id = agent_id.clone();
        let prompt = config.prompt.clone();
        async move {
            let result = run_agent(RunAgentParams {
                agent_def,
                system_prompt: get_system_prompt(),
                user_context: Default::default(),
                system_context: Default::default(),
                tool_context: parent_context.create_subagent_context(None),
                can_use_tool: Box::new(|_, _| Ok(())),
                query_source: QuerySource::Agent,
                messages: vec![Message::User {
                    content: ContentBlock::Text(prompt),
                    ..Default::default()
                }],
                max_turns: None,
                allowed_tools: None,
                model_override: Some(model),
                on_cache_safe_params: None,
            }).await;

            // 汇聚结果到消息或日志
            match result {
                Ok(_) => {
                    // 记录成功
                },
                Err(e) => {
                    eprintln!("Agent {} failed: {}", agent_id, e);
                }
            }
        }
    });

    Ok(agent_id)
}

async fn generate_unique_name(
    base_name: &str,
    team_name: Option<&str>,
) -> Result<String, AgentError> {
    // 如果无团队，直接返回基名称
    if team_name.is_none() {
        return Ok(base_name.to_string());
    }

    // 检查团队成员是否有重复
    let team_file = load_team_file(team_name.unwrap()).await?;
    let existing = team_file
        .members
        .iter()
        .map(|m| m.name.to_lowercase())
        .collect::<std::collections::HashSet<_>>();

    if !existing.contains(&base_name.to_lowercase()) {
        return Ok(base_name.to_string());
    }

    // 追加数字后缀
    for suffix in 2..=100 {
        let candidate = format!("{}-{}", base_name, suffix);
        if !existing.contains(&candidate.to_lowercase()) {
            return Ok(candidate);
        }
    }

    Err(AgentError::NameGeneration(base_name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_agent_basic() {
        let config = SpawnAgentConfig {
            name: "researcher".to_string(),
            prompt: "Analyze the code".to_string(),
            team_name: Some("my-team".to_string()),
            agent_type: None,
            model: None,
            cwd: None,
        };
        
        let parent = create_test_context();
        let agent_id = spawn_agent_in_team(config, &parent).await.unwrap();
        
        assert!(!agent_id.as_str().is_empty());
    }

    #[tokio::test]
    async fn test_unique_name_generation() {
        let name1 = generate_unique_name("researcher", None).await.unwrap();
        assert_eq!(name1, "researcher");

        // 模拟团队有相同名称
        let name2 = generate_unique_name("researcher", Some("my-team")).await.unwrap();
        // 应该返回 researcher-2 或类似的
        assert!(name2.starts_with("researcher"));
    }
}
```

---

## 集成检查清单

### 编译和单元测试
- [ ] Phase 1.1：`cargo test -p sagec agent::definition`
- [ ] Phase 1.2：`cargo test -p sagec agent::builtin`
- [ ] Phase 2.1：`cargo test -p sagec context`
- [ ] Phase 2.2：`cargo test -p sagec agent::forked`
- [ ] Phase 3.1：`cargo test -p sagec agent::runner`
- [ ] Phase 3.2：`cargo test -p sagec agent::query_loop`
- [ ] Phase 4.1：`cargo test -p sagec team`

### 集成测试
- [ ] End-to-end：single agent execution
- [ ] Fork mechanism：implicit agent inheritance
- [ ] Team creation：parallel agent spawning
- [ ] Permission checking：tool access control
- [ ] MCP services：lifecycle management
- [ ] Prompt caching：cache-safe parameter passing

### 文档
- [ ] API 文档：`cargo doc --open`
- [ ] 迁移指南：CC → Rust mapping
- [ ] 故障排除：常见问题和解决方案

---

## 性能优化建议

### 1. 消息克隆成本
```rust
// 使用 Cow<> 避免不必要的克隆
pub struct Message {
    content: Cow<'a, [ContentBlock]>,  // 共享或拥有
    ...
}
```

### 2. FileStateCache 管理
```rust
// 预分配缓存大小，避免动态增长
pub struct FileStateCache {
    cache: DashMap<PathBuf, FileState>,  // 线程安全哈希表
    max_size: usize,  // 限制大小
}
```

### 3. Agent 定义缓存
```rust
// 缓存加载的 agent 定义，避免重复解析
pub static AGENT_DEF_CACHE: Lazy<DashMap<String, AgentDef>> = 
    Lazy::new(DashMap::new);
```

### 4. 异步流处理
```rust
// 使用 async_stream 宏生成高效的 stream
pub async fn run_agent(...) -> impl Stream<Item = Message> {
    async_stream::stream! {
        // 逐个 yield 消息，避免全部加载到内存
    }
}
```

---

## 已知限制和未来工作

### v1.0 限制
1. **不支持**：MCP 服务动态配置（仅支持预定义服务）
2. **不支持**：Tmux 多窗格管理（可后续添加）
3. **不支持**：转录持久化（可由上层应用实现）

### v2.0 方向
1. 分布式 agent（多进程/多机）
2. Agent 计算图（DAG 调度）
3. 缓存感知调度（最大化 prompt cache 命中）
4. 可观测性增强（追踪、指标、日志）

---

## 工作量估算

| Phase | 任务 | 复杂度 | 工作量 |
|-------|------|--------|--------|
| 1.1 | AgentDef + 序列化 | 低 | 4 小时 |
| 1.2 | 内置 agents 注册 | 低 | 2 小时 |
| 2.1 | ToolContext 隔离 | 中 | 6 小时 |
| 2.2 | ForkedAgent + Fork 消息 | 中 | 6 小时 |
| 3.1 | RunAgent 异步流 | 高 | 8 小时 |
| 3.2 | Query 循环 + 工具执行 | 高 | 10 小时 |
| 4.1 | 团队管理（可选） | 中 | 6 小时 |
| 测试 | 单元 + 集成测试 | 中 | 8 小时 |
| 文档 | API 文档 + 指南 | 低 | 4 小时 |
| **总计** | - | - | **54 小时（1.5 周）** |

---

## 交付标志

✅ **阶段 1：定义和加载**
- AgentDef 序列化完整
- 6 个内置 agent 注册
- 单元测试覆盖

✅ **阶段 2：上下文隔离**
- ToolContext 克隆策略清晰
- Fork 消息处理正确
- CacheSafeParams 构建可靠

✅ **阶段 3：核心执行**
- Query 循环稳定运行
- 工具执行通过权限检查
- 流式消息返回可用

✅ **阶段 4：集成和文档**
- 端到端测试通过
- 性能基准确定
- API 文档完整

---

**预计交付日期**：2026-05-05  
**负责人**：@sage-research  
**审核人**：@impl-lead
