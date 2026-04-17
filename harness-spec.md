# Sage v0.1.0 — SageEngine Builder API & CLI 改造

## 概述

将 Sage 从"手工组装才能跑"改造为"3 行代码跑起 AI Agent"的可嵌入执行引擎。核心交付物是 `SageEngine` builder API（sage-runtime crate）和 `sage run` CLI 子命令（sage-cli crate）。

## 目标

- **一句话**：让外部用户用 builder pattern 3 行代码创建并运行一个 AI Agent，无需了解内部组装细节
- **定位**："SQLite of AI Agents" — 零配置嵌入，单 crate 依赖

## Non-Goals

- 不涉及沙箱（microVM）集成 — SageEngine 在 host 进程内直接执行工具
- 不涉及 Rune SDK 集成 — `sage serve` 命令保持 stub 模式
- 不引入新的 trait — `SageTool` 是 `AgentTool` 的 re-export，不是新 trait
- 不修改 Agent/AgentLoop/EventStream 的内部实现
- 不添加 HTTP/gRPC server 能力

## 成功标准

1. 用户添加 `sage-runtime` 依赖后，可以用以下代码跑通 Agent 循环：
   ```rust
   let engine = SageEngine::builder()
       .system_prompt("你是一个代码助手")
       .provider("qwen")
       .model("qwen-plus")
       .max_turns(10)
       .register_tool(MyTool)
       .build()?;
   let mut rx = engine.run("hello").await?;
   while let Some(event) = rx.next().await { /* ... */ }
   ```
2. `cargo run -p sage -- run --config configs/coding-assistant.yaml --message "hello"` 等价于当前 `--local-test`
3. 所有 876 个现有测试继续通过
4. 新增测试覆盖 builder/engine/CLI 约 20-25 个

---

## P1: SageEngine Builder API

**优先级**：P0（必须交付）
**位置**：`crates/sage-runtime/src/engine.rs`，新增 `pub mod engine` 到 `lib.rs`

### 架构决策

| # | 决策 | 理由 |
|---|------|------|
| 1 | `pub use AgentTool as SageTool` | 不引入新 trait，减少概念负担 |
| 2 | SageEngine 持有原子字段，不持有 AgentConfig | sage-runtime 不依赖 sage-runner，避免循环依赖；每次 `run()` 从原子字段组装 Agent + Provider + Registry |
| 3 | `ChannelSink` 实现 `AgentEventSink` | 桥接 agent_loop 事件到 EventStream channel |
| 4 | `.llm_provider()` 支持注入自定义 LlmProvider | 测试用 StatefulProvider mock；高级用户可替换 LLM 后端 |
| 5 | `resolve_or_construct_model()` 接受原子参数 | 与 llm/models.rs 的 `resolve_model()` 区分，包含 fallback 构造逻辑 |
| 6 | `StatefulProvider` 从 agent_loop.rs 测试提取到 test_helpers.rs | engine.rs 测试需要复用 mock provider |

### 依赖架构

```
sage-runner ──depends──▶ sage-runtime
sage-cli ──depends──▶ sage-runner + sage-runtime

SageEngine 定义在 sage-runtime，不依赖 sage-runner
sage-cli 做 YAML → AgentConfig → SageEngineBuilder 原子字段的胶水
```

`sage-runtime` 的 `Cargo.toml` 新增：`thiserror`（SageError derive）。不新增对 sage-runner 的依赖。

### 数据模型

```rust
// crates/sage-runtime/src/engine.rs

/// Sage 执行引擎 — 持有原子配置字段，每次 run() 创建新的 Agent 实例。
/// 不持有 AgentConfig（来自 sage-runner），避免循环依赖。
pub struct SageEngine {
    // ── Agent 配置 ──
    system_prompt: String,
    max_turns: usize,                                  // 默认 10
    tool_execution_mode: ToolExecutionMode,             // 默认 Parallel
    tool_policy: Option<ToolPolicy>,

    // ── 工具 ──
    builtin_tool_names: Vec<String>,                   // 内置工具名（bash, read, write...）
    extra_tools: Vec<Arc<dyn AgentTool>>,               // 用户自定义工具（Arc 因为多次 run() 需要共享）

    // ── LLM ──
    provider_name: String,                             // e.g., "qwen", "deepseek"
    model_id: String,                                  // e.g., "qwen-plus"
    max_tokens: u32,                                   // 默认 4096
    base_url: Option<String>,                          // 覆盖模型 base URL
    api_key_env: Option<String>,                       // 覆盖 API key 环境变量名
    custom_llm_provider: Option<Arc<dyn LlmProvider>>, // 注入的自定义 provider

    // ── Hooks ──
    before_hook: Option<Arc<dyn BeforeToolCallHook>>,
    after_hook: Option<Arc<dyn AfterToolCallHook>>,
}

/// Builder — 链式配置 SageEngine
pub struct SageEngineBuilder {
    system_prompt: Option<String>,
    max_turns: Option<usize>,
    tool_execution_mode: Option<ToolExecutionMode>,
    tool_policy: Option<ToolPolicy>,
    builtin_tool_names: Vec<String>,
    extra_tools: Vec<Arc<dyn AgentTool>>,
    provider_name: Option<String>,
    model_id: Option<String>,
    max_tokens: Option<u32>,
    base_url: Option<String>,
    api_key_env: Option<String>,
    custom_llm_provider: Option<Arc<dyn LlmProvider>>,
    before_hook: Option<Arc<dyn BeforeToolCallHook>>,
    after_hook: Option<Arc<dyn AfterToolCallHook>>,
}

/// ChannelSink — 桥接 AgentEventSink → EventSender
struct ChannelSink {
    sender: EventSender<AgentEvent, Vec<AgentMessage>>,
}
```

### build() 必填与默认字段

**必填字段**（缺少任一则 `build()` 返回 `SageError::MissingField`）：

| 字段 | 设置方式 | 说明 |
|------|---------|------|
| `system_prompt` | `.system_prompt("...")` | Agent 系统提示词 |
| LLM provider | `.provider("qwen") + .model("qwen-plus")` **或** `.llm_provider(impl LlmProvider)` | 二选一：字符串选择 或 注入实例 |

**可选字段**（有合理默认值）：

| 字段 | 默认值 | 设置方式 |
|------|--------|---------|
| `max_turns` | `10` | `.max_turns(n)` |
| `max_tokens` | `4096` | `.max_tokens(n)` |
| `tool_execution_mode` | `Parallel` | `.tool_execution_mode(mode)` |
| `tool_policy` | `None`（不限制） | `.tool_policy(policy)` |
| `builtin_tool_names` | `[]`（无内置工具） | `.builtin_tools(["bash", "read"])` |
| `extra_tools` | `[]` | `.register_tool(tool)` |
| `base_url` | `None`（使用 catalog 默认） | `.base_url("...")` |
| `api_key_env` | `None`（使用 catalog 默认） | `.api_key_env("...")` |
| `before_hook` | `None` | `.on_before_tool_call(hook)` |
| `after_hook` | `None` | `.on_after_tool_call(hook)` |

**build() 校验逻辑**：
1. `system_prompt` 未设置 → `SageError::MissingField("system_prompt")`
2. `custom_llm_provider` 和 (`provider_name` + `model_id`) 都未设置 → `SageError::MissingField("provider+model or llm_provider")`
3. 如果 `custom_llm_provider` 已设置，`provider_name`/`model_id` 可选（但 `model_id` 仍需要一个值用于 AgentLoopConfig，如果未设置则默认 `"custom"`）

### API 签名

```rust
impl SageEngine {
    /// 创建 builder
    pub fn builder() -> SageEngineBuilder;

    /// 执行 Agent 循环，返回事件接收器。
    /// 每次调用创建新的 Agent、Provider、ToolRegistry。
    pub async fn run(&self, message: &str) -> Result<EventReceiver<AgentEvent, Vec<AgentMessage>>, SageError>;
}

impl SageEngineBuilder {
    // ── Agent 配置 ──

    /// 设置系统提示词（必填）
    pub fn system_prompt(self, prompt: &str) -> Self;

    /// 设置最大轮次（默认 10）
    pub fn max_turns(self, n: usize) -> Self;

    /// 设置工具执行模式（默认 Parallel）
    pub fn tool_execution_mode(self, mode: ToolExecutionMode) -> Self;

    /// 设置工具策略（白名单校验）
    pub fn tool_policy(self, policy: ToolPolicy) -> Self;

    // ── 工具 ──

    /// 设置内置工具列表（从 config 工具名解析，e.g., ["bash", "read", "write"]）
    pub fn builtin_tools(self, names: &[&str]) -> Self;

    /// 注册自定义工具（追加到内置工具之后）
    pub fn register_tool(self, tool: impl AgentTool + 'static) -> Self;

    // ── LLM ──

    /// 设置 LLM provider 名称（e.g., "qwen", "deepseek"）
    /// 与 .model() 配合使用，run() 时通过 resolve_or_construct_model() 解析
    pub fn provider(self, provider: &str) -> Self;

    /// 设置模型 ID（e.g., "qwen-plus", "deepseek-chat"）
    pub fn model(self, model: &str) -> Self;

    /// 设置 max_tokens（默认 4096）
    pub fn max_tokens(self, n: u32) -> Self;

    /// 覆盖模型 base URL
    pub fn base_url(self, url: &str) -> Self;

    /// 覆盖 API key 环境变量名
    pub fn api_key_env(self, env_var: &str) -> Self;

    /// 注入自定义 LLM provider（高级用法 + 测试用）。
    /// 设置后 run() 直接使用此 provider，忽略 .provider()/.model() 的字符串选择。
    pub fn llm_provider(self, provider: impl LlmProvider + 'static) -> Self;

    // ── Hooks ──

    /// 注册 before tool call hook
    pub fn on_before_tool_call(self, hook: impl BeforeToolCallHook + 'static) -> Self;

    /// 注册 after tool call hook
    pub fn on_after_tool_call(self, hook: impl AfterToolCallHook + 'static) -> Self;

    // ── Build ──

    /// 构建 SageEngine。
    /// 必填：system_prompt + (provider+model 或 llm_provider)
    pub fn build(self) -> Result<SageEngine, SageError>;
}
```

### 错误类型

```rust
// crates/sage-runtime/src/engine.rs

#[derive(Debug, thiserror::Error)]
pub enum SageError {
    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("model resolution failed: {0}")]
    ModelResolution(String),

    #[error("agent loop error: {0}")]
    AgentLoop(#[from] AgentLoopError),
}
```

### run() 内部流程

```
run(message)
  ├── 1. 创建 LLM provider:
  │      if custom_llm_provider.is_some() → Arc::clone(custom_llm_provider)
  │      else → resolve_or_construct_model(provider_name, model_id, ...) → OpenAiCompatProvider::new()
  ├── 2. ToolRegistry::new()
  │      + register builtin tools (from builtin_tool_names via create_tool())
  │      + register extra_tools (Arc::clone each)
  ├── 3. AgentLoopConfig { model, system_prompt, max_turns, tool_execution_mode, tool_policy }
  ├── 4. Agent::new(loop_config, provider, registry)
  ├── 5. agent.set_before/after_tool_call(Arc::clone hooks)  // if set
  ├── 6. agent.steer(UserMessage::from_text(message))
  ├── 7. EventStream::new() → (sender, receiver)
  ├── 8. ChannelSink { sender }
  ├── 9. tokio::spawn { run_agent_loop(agent, &sink) → sink.sender.end(result) }
  └── 10. return receiver
```

### 公共 re-exports

在 `crates/sage-runtime/src/lib.rs` 添加：

```rust
pub mod engine;

// 顶层 re-export（用户不需要深入子模块）
pub use engine::{SageEngine, SageEngineBuilder, SageError};
pub use tools::AgentTool as SageTool;
pub use event::{AgentEvent, EventReceiver};
```

### resolve_or_construct_model

从 `serve.rs:124-173` 提取模型解析逻辑到 `engine.rs`，改为接受原子参数（不依赖 AgentConfig）。

与 `llm/models.rs` 已有的 `resolve_model(provider, model_id) -> Option<Model>` 区分：
- `resolve_model()` — 仅查 catalog，返回 Option
- `resolve_or_construct_model()` — 先查 catalog，miss 时用提供的字段构造 Model

```rust
/// 解析或构造 Model。
/// 先尝试内置 catalog（resolve_model），miss 时从提供的参数构造。
pub fn resolve_or_construct_model(
    provider: &str,
    model_id: &str,
    max_tokens: u32,
    base_url: Option<&str>,
    api_key_env: Option<&str>,
) -> Result<Model, SageError> {
    // 1. Try built-in catalog
    if let Some(mut model) = models::resolve_model(provider, model_id) {
        // Apply overrides if provided
        if let Some(url) = base_url {
            model.base_url = url.to_string();
        }
        if let Some(env) = api_key_env {
            model.api_key_env = env.to_string();
        }
        return Ok(model);
    }

    // 2. Catalog miss — construct from provided fields
    let url = base_url.ok_or_else(|| {
        SageError::ModelResolution(format!(
            "model '{model_id}' not in catalog; base_url required"
        ))
    })?;

    Ok(Model {
        id: model_id.into(),
        name: model_id.into(),
        api: api::OPENAI_COMPLETIONS.into(),
        provider: provider.into(),
        base_url: url.to_string(),
        api_key_env: api_key_env
            .map(|s| s.to_string())
            .unwrap_or_else(|| keys::api_key_env_var(provider)),
        reasoning: false,
        input: vec![InputType::Text],
        max_tokens,
        context_window: 128000,
        cost: ModelCost::default(),
        headers: vec![],
        compat: Some(ProviderCompat::default()),
    })
}
```

`serve.rs` 改为调用 `sage_runtime::engine::resolve_or_construct_model()`，传入从 AgentConfig 解构的原子字段。

### StatefulProvider 提取

将 `agent_loop.rs` 测试中的 `StatefulProvider`（约 L550-566）提取到 `test_helpers.rs`，使其可被 `engine.rs` 的测试复用。

当前 `test_helpers.rs` 仅包含 `test_model()` 和 `test_context()`，追加：

```rust
// crates/sage-runtime/src/test_helpers.rs（#[cfg(test)] 模块）

pub struct StatefulProvider {
    responses: Mutex<VecDeque<Vec<AssistantMessageEvent>>>,
    call_count: AtomicUsize,
}

impl StatefulProvider {
    pub fn new(responses: Vec<Vec<AssistantMessageEvent>>) -> Self { ... }
    pub fn call_count(&self) -> usize { ... }
}

#[async_trait]
impl LlmProvider for StatefulProvider { ... }
```

`agent_loop.rs` 测试改为 `use crate::test_helpers::StatefulProvider`。

---

## P2: CLI 子命令改造

**优先级**：P1（应该交付）
**位置**：`crates/sage-cli/src/main.rs`、`crates/sage-cli/src/serve.rs`

### 当前 CLI 结构

```
sage [--local-test] [--config X] [--message Y] [--provider Z] [--model W]
sage [--runtime addr] [--caster-id id]
```

### 目标 CLI 结构

```
sage run --config X --message Y [--provider Z] [--model W]
sage serve --runtime addr [--caster-id id] [--max-concurrent N]
```

### main.rs 实现

```rust
// crates/sage-cli/src/main.rs

#[derive(Parser)]
#[command(name = "sage", about = "Sage — embeddable AI agent execution engine")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run an agent locally: load config -> execute -> print result
    Run {
        /// Path to agent config YAML
        #[arg(long)]
        config: String,

        /// Message to send to the agent
        #[arg(long)]
        message: String,

        /// Override LLM provider (e.g., qwen, deepseek)
        #[arg(long)]
        provider: Option<String>,

        /// Override model ID (e.g., qwen-plus, deepseek-chat)
        #[arg(long)]
        model: Option<String>,
    },

    /// Start as a Rune Caster service (Phase 2)
    Serve {
        /// Rune Runtime gRPC address
        #[arg(long, default_value = "localhost:50070")]
        runtime: String,

        /// Caster ID for Rune registration
        #[arg(long, default_value = "agents-executor")]
        caster_id: String,

        /// Max concurrent sandbox VMs
        #[arg(long, default_value = "3")]
        max_concurrent: usize,
    },
}
```

### serve.rs 改造

- `run_local_test()` + `handle_execute()` 改为使用 `SageEngine` API
- 删除 `resolve_model_from_config()`（逻辑已移至 engine.rs 的 `resolve_or_construct_model`）
- `TerminalEventSink` 替换为消费 `EventReceiver` 的循环 + `print_event()` 函数

改造后的核心逻辑：

```rust
// sage-cli/src/serve.rs
use sage_runner::AgentConfig;
use sage_runtime::engine::SageEngine;

pub async fn run_local_test(
    config_path: &str,
    message: &str,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Result<()> {
    let yaml = tokio::fs::read_to_string(config_path).await?;
    let config: AgentConfig = serde_yaml::from_str(&yaml)?;

    // AgentConfig → SageEngineBuilder 原子字段（胶水逻辑在 sage-cli 层）
    let mut builder = SageEngine::builder()
        .system_prompt(&config.system_prompt)
        .provider(provider_override.unwrap_or(&config.llm.provider))
        .model(model_override.unwrap_or(&config.llm.model))
        .max_tokens(config.llm.max_tokens)
        .max_turns(config.constraints.max_turns as usize)
        .tool_execution_mode(ToolExecutionMode::Parallel)
        .tool_policy(config.tools.to_policy())
        .builtin_tools(&config.tools.tool_names().iter().map(|s| s.as_str()).collect::<Vec<_>>());

    if let Some(url) = &config.llm.base_url {
        builder = builder.base_url(url);
    }
    if let Some(env) = &config.llm.api_key_env {
        builder = builder.api_key_env(env);
    }

    let engine = builder.build()?;
    let mut rx = engine.run(message).await?;
    while let Some(event) = rx.next().await {
        print_event(&event);
    }
    Ok(())
}

fn print_event(event: &AgentEvent) {
    // 原 TerminalEventSink::emit() 的逻辑，改为同步打印
    // ...
}
```

### 向后兼容

- `--local-test` 参数移除（breaking change，v0.1.0 允许）
- 用户改用 `sage run --config X --message Y`

---

## 测试策略

### P1 SageEngine 测试（~21 个，crates/sage-runtime/src/engine.rs #[cfg(test)]）

所有 Engine 运行测试通过 `.llm_provider(StatefulProvider::new(...))` 注入 mock provider。

**Builder 测试（~9 个）**：

| # | 测试名 | 验证内容 |
|---|--------|---------|
| 1 | `builder_default_values` | 默认 builder 字段均为 None/empty |
| 2 | `builder_minimal_build_succeeds` | 设置 system_prompt + provider + model 后 build 成功 |
| 3 | `builder_missing_system_prompt_fails` | 未设置 system_prompt → `SageError::MissingField` |
| 4 | `builder_missing_provider_and_llm_provider_fails` | 未设置 provider+model 且未设置 llm_provider → `SageError::MissingField` |
| 5 | `builder_llm_provider_without_provider_name_succeeds` | 设置 llm_provider 但不设置 provider/model → build 成功 |
| 6 | `builder_register_tool` | 注册自定义工具后 build 成功 |
| 7 | `builder_hooks_registered` | .on_before/after_tool_call() 注册 hook 后 build 成功 |
| 8 | `builder_multiple_tools` | 注册多个自定义工具 |
| 9 | `builder_chaining` | 链式调用所有 builder 方法 |

**Engine 运行测试（~9 个，通过 .llm_provider() 注入 StatefulProvider mock）**：

| # | 测试名 | 验证内容 |
|---|--------|---------|
| 10 | `run_emits_agent_start_and_end` | 事件流包含 AgentStart 和 AgentEnd |
| 11 | `run_stream_terminates` | next() 最终返回 None |
| 12 | `run_returns_final_messages` | receiver.result() 包含 Agent 输出 |
| 13 | `run_with_tool_calls` | 工具调用产生 ToolExecution* 事件 |
| 14 | `run_multiple_times` | 同一 Engine 多次 run() 互不干扰 |
| 15 | `run_hook_blocks_tool` | before hook 阻止工具执行 |
| 16 | `run_max_turns_respected` | 达到 max_turns 后循环终止 |
| 17 | `run_unknown_tool_error` | LLM 请求未注册工具时产生错误事件 |
| 18 | `run_custom_tool_executes` | 自定义工具被正确调用并返回结果 |

**resolve_or_construct_model 测试（~3 个）**：

| # | 测试名 | 验证内容 |
|---|--------|---------|
| 19 | `resolve_catalog_hit` | 内置 catalog 模型正确解析 |
| 20 | `resolve_catalog_miss_with_base_url` | 非 catalog 模型 + base_url 构造成功 |
| 21 | `resolve_catalog_miss_no_base_url_fails` | 非 catalog 模型无 base_url → `SageError::ModelResolution` |

### P2 CLI 测试（~3 个，crates/sage-cli/tests/ 或 src/main.rs）

| # | 测试名 | 验证内容 |
|---|--------|---------|
| 1 | `cli_run_subcommand_parses` | `sage run --config X --message Y` 正确解析 |
| 2 | `cli_serve_subcommand_parses` | `sage serve --runtime X` 正确解析 |
| 3 | `cli_run_with_overrides` | `--provider` 和 `--model` 覆盖参数解析 |

---

## 实施顺序

```
Step 1: 提取 StatefulProvider → test_helpers.rs
        - 从 agent_loop.rs L550-566 移动到 test_helpers.rs
        - agent_loop.rs 测试改为 use crate::test_helpers::StatefulProvider
        - 验证：cargo test -p sage-runtime 全部通过

Step 2: 实现 engine.rs（SageEngine + SageEngineBuilder + ChannelSink + SageError）
        - 包含 resolve_or_construct_model()
        - 包含所有 ~21 个测试
        - 更新 lib.rs 添加 pub mod engine + re-exports
        - 验证：cargo test -p sage-runtime 全部通过

Step 3: 改造 sage-cli（子命令 + serve.rs 使用 SageEngine）
        - main.rs 改为 subcommand 结构
        - serve.rs 改用 SageEngine builder API（AgentConfig → 原子字段胶水）
        - 删除 serve.rs 中的 resolve_model_from_config()
        - 添加 CLI 解析测试
        - 验证：cargo test --workspace 全部通过

Step 4: 最终验证
        - cargo test --workspace（876 + 新增 ~24 个测试全部通过）
        - cargo run -p sage -- run --config configs/coding-assistant.yaml --message "hello"
```

---

## 验收标准

### P1 SageEngine

- [ ] `SageEngine::builder().system_prompt("...").provider("test").model("test").build()` 编译通过
- [ ] `SageEngine::builder().system_prompt("...").llm_provider(mock).build()` 编译通过（注入自定义 provider）
- [ ] `.build()` 缺少 system_prompt 返回 `SageError::MissingField`
- [ ] `.build()` 缺少 provider+model 且无 llm_provider 返回 `SageError::MissingField`
- [ ] `engine.run("hello").await` 返回 `EventReceiver`，可通过 `.next().await` 消费事件
- [ ] 同一 Engine 实例多次调用 `run()` 互不干扰
- [ ] 自定义工具通过 `.register_tool()` 注册后可被 Agent 使用
- [ ] Before/After hooks 通过 builder 注册后在工具调用时触发
- [ ] `resolve_or_construct_model()` 作为公共函数可从外部调用，接受原子参数
- [ ] 所有新增测试通过，现有 876 个测试不回归

### P2 CLI

- [ ] `sage run --config X --message Y` 替代 `sage --local-test --config X --message Y`
- [ ] `sage serve --runtime addr` 保持原有行为
- [ ] `--provider` 和 `--model` 覆盖参数在 `sage run` 中工作
- [ ] serve.rs 中不再有 `resolve_model_from_config()` 的独立拷贝
