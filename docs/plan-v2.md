# Sage v1.0.0 开发计划

**目标**：`sage chat --agent <name>` 完整跑通多轮 TUI 交互，含 Agent 注册、持久化工作空间、记忆自动注入。

---

## Agent 完整数据结构

### `agent.yaml` 字段规范

```yaml
# ~/.sage/agents/<name>/agent.yaml

# ── Identity ──────────────────────────────────────────
name: feishu                        # string, 必填，唯一标识（=目录名）
description: "飞书专员 — 管理我的飞书工作空间"  # string, 必填
version: "1.0.0"                    # string, 可选，默认 "1.0.0"

# ── LLM ───────────────────────────────────────────────
llm:
  provider: anthropic                # string, 必填（anthropic|openai|google|deepseek|...）
  model: claude-sonnet-4-6           # string, 必填
  max_tokens: 8192                   # u32, 必填
  base_url: null                     # string | null, 可选，覆盖 provider 默认 URL
  api_key_env: null                  # string | null, 可选，覆盖默认的 env var 名

# ── System Prompt ─────────────────────────────────────
system_prompt: |                     # string, 必填（多行文本）
  你是我的飞书专员。你有一个独立的工作空间 /workspace，
  可以在里面写代码、执行脚本、保存文件和积累知识。
  ...

# ── Tools ─────────────────────────────────────────────
tools:
  bash:
    enabled: true                    # bool, 默认 true（如字段存在）
    allowed_binaries:                # string[], 白名单。空列表 = 禁止所有
      - python3
      - node
      - curl
      - jq
      - git
      - rg
      - fd
  read:
    enabled: true
    allowed_paths:                   # string[], 允许读取的路径前缀
      - "/workspace"
  write:
    enabled: true
    allowed_paths:                   # string[], 允许写入的路径前缀
      - "/workspace"
  edit:
    enabled: true
    allowed_paths:
      - "/workspace"
  grep:
    enabled: true
  find:
    enabled: true
  ls:
    enabled: true

# ── Sandbox ───────────────────────────────────────────
sandbox:
  cpus: 2                            # u32, 默认 1
  memory_mib: 2048                   # u32, 默认 256
  network: full                      # airgapped | full | whitelist，默认 airgapped
  workspace_host: "~/.sage/agents/feishu/workspace"
                                     # string | null, 挂载到 VM 的 /workspace
                                     # ~ 自动展开，默认 null（不挂载持久化目录）
  security:                          # 可选，省略则用默认值
    seccomp: true
    landlock: true
    max_file_size_mb: 100
    max_open_files: 1024
    tmpfs_size_mb: 512
    max_processes: 64

# ── Constraints ───────────────────────────────────────
constraints:
  max_turns: 50                      # u32, 默认 30
  timeout_secs: 600                  # u64, 默认 300

# ── Memory ────────────────────────────────────────────
memory:
  auto_load:                         # string[], 相对 workspace 根目录的文件路径
    - MEMORY.md                      # 不存在时静默跳过（首次启动正常）
    - CONVENTIONS.md
  inject_as: prepend_system          # prepend_system | initial_message
                                     # prepend_system: 追加到 system_prompt 末尾
                                     # initial_message: 作为首条 user/assistant 消息对注入
```

### Rust 结构体（sage-runner/src/config.rs 新增部分）

```rust
// 新增：MemoryConfig
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryConfig {
    #[serde(default)]
    pub auto_load: Vec<String>,
    #[serde(default)]
    pub inject_as: MemoryInjectMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryInjectMode {
    #[default]
    PrependSystem,
    InitialMessage,
}

// 修改：AgentConfig 加字段
pub struct AgentConfig {
    pub name: String,
    pub description: String,
    #[serde(default = "default_version")]
    pub version: String,
    pub llm: LlmConfig,
    pub system_prompt: String,
    pub tools: ToolsConfig,
    pub constraints: Constraints,
    #[serde(default)]
    pub sandbox: Option<SandboxConfig>,
    #[serde(default)]
    pub memory: MemoryConfig,         // 新增
}

// 修改：SandboxConfig 加字段
pub struct SandboxConfig {
    // ... 现有字段 ...
    #[serde(default, deserialize_with = "deserialize_tilde_path")]
    pub workspace_host: Option<PathBuf>,  // 新增
}
```

---

## Agent 目录规范

```
~/.sage/
├── config.toml          # 全局配置（可选，优先级低于 agent.yaml）
└── agents/
    └── <name>/
        ├── agent.yaml   # 唯一声明文件（静态配置，不由 Agent 自身修改）
        └── workspace/   # 持久化工作空间，mount 到 VM /workspace
            ├── MEMORY.md        # 事实性知识（API 路径、ID、规律）
            ├── CONVENTIONS.md   # 用户偏好与规范
            └── ...              # Agent 自由创建的文件
```

**`agent.yaml` 不在工作空间内，Agent 在 VM 内无法访问**（设计决定，见下文 M1 讨论）。

---

## M1：Agent 注册与发现

### 功能设计

Agent 注册 = 在 `~/.sage/agents/<name>/` 下创建 `agent.yaml` + `workspace/`。

**`sage init --agent <name>`** 交互式生成模板：

```
$ sage init --agent feishu

Creating agent: feishu
Description (e.g. "飞书专员 — 管理我的飞书工作空间"):
> 飞书专员 — 管理我的飞书工作空间

LLM provider [anthropic/openai/deepseek] (default: anthropic):
> anthropic

Model (default: claude-sonnet-4-6):
> (Enter 跳过)

Network access [airgapped/full] (default: airgapped):
> full

✓ Created ~/.sage/agents/feishu/agent.yaml
✓ Created ~/.sage/agents/feishu/workspace/
✓ Created ~/.sage/agents/feishu/workspace/MEMORY.md (empty)
✓ Created ~/.sage/agents/feishu/workspace/CONVENTIONS.md (empty)

Next: edit ~/.sage/agents/feishu/agent.yaml to customize system_prompt and tools.
Then: sage chat --agent feishu
```

**`sage list`** 输出所有已注册 Agent：

```
$ sage list

NAME            DESCRIPTION                           LLM
feishu          飞书专员 — 管理我的飞书工作空间         anthropic/claude-sonnet-4-6
github-review   代码审查专员                          anthropic/claude-haiku-4-5
research        信息调研专员                          openai/gpt-4o
```

**`sage validate --agent <name>`** 校验配置合法性（YAML 解析 + 字段检查）。

### 方案逻辑

Agent 发现：扫描 `~/.sage/agents/*/agent.yaml`，解析每个文件的 `name/description/llm`。

`--agent <name>` 解析路径：

```
~/.sage/agents/<name>/agent.yaml
```

如果 `<name>` 包含 `/`（如 `./configs/feishu.yaml`），视为直接路径（backward compatible 兼容旧 `--config`）。

### To-Do

- [ ] `sage-cli/src/registry.rs` — 新文件
  - `fn sage_dir() -> PathBuf` → `~/.sage/`
  - `fn agents_dir() -> PathBuf` → `~/.sage/agents/`
  - `fn agent_dir(name: &str) -> PathBuf`
  - `fn agent_config_path(name: &str) -> PathBuf`
  - `fn agent_workspace_path(name: &str) -> PathBuf`
  - `fn list_agents() -> Vec<AgentEntry>`（扫描 + 解析）
  - `fn resolve_agent(name_or_path: &str) -> Result<(AgentConfig, PathBuf)>`
- [ ] `sage-cli/src/init.rs` — 新文件
  - `fn run_init(name: &str) -> Result<()>`（交互式 CLI wizard）
  - 生成 `agent.yaml` 模板（根据用户输入填充）
  - 创建 `workspace/`、`MEMORY.md`（空）、`CONVENTIONS.md`（空）
- [ ] `sage-cli/src/main.rs` — 添加子命令
  - `Init { name: String }`
  - `List`
  - `Validate { agent: String }`
- [ ] 单元测试：`registry::list_agents` 对 fixture 目录的扫描

---

## M2：Workspace 持久化挂载

### 功能设计

`agent.yaml` 中 `sandbox.workspace_host` 指向主机目录，Sage 启动时将其 mount 到 VM 的 `/workspace`。

- `sandbox.workspace_host` 支持 `~` 展开
- 目录不存在时自动创建
- `network: full` 时主机网络透传进 VM

### 方案逻辑

`SandboxBuilder` 已有 `volumes: Vec<VolumeMount>`，只需在启动前注入：

```rust
// sage-cli/src/chat.rs 或 serve.rs 中，组装 SandboxBuilder 时

if let Some(ref sandbox_cfg) = config.sandbox {
    if let Some(ref host_path) = sandbox_cfg.workspace_host {
        // 目录不存在时自动创建
        std::fs::create_dir_all(host_path)?;
        builder = builder.volume(VolumeMount {
            host_path: host_path.to_string_lossy().into_owned(),
            guest_path: "/workspace".to_string(),
            read_only: false,
        });
    }
}
```

### 依赖确认

- `sage-sandbox/builder.rs` 中 `VolumeMount` 和 `volumes` 字段已存在 ✓
- `SandboxConfig` 中加 `workspace_host: Option<PathBuf>` 字段
- `config.rs` 中加 `deserialize_tilde_path` 反序列化器（展开 `~`）

### To-Do

- [ ] `sage-runner/src/config.rs`
  - `SandboxConfig` 加 `workspace_host: Option<PathBuf>`
  - 加 `fn deserialize_tilde_path` serde 反序列化器（复用已有 `expand_tilde`）
- [ ] `sage-cli/src/sandbox_setup.rs`（或内联到 chat.rs）
  - `fn apply_workspace_mount(builder, config) -> SandboxBuilder`
  - 自动 `create_dir_all` + 注入 VolumeMount
- [ ] 集成测试：启动沙箱 → 在 `/workspace` 写文件 → VM 关闭 → 文件留在主机

---

## M3：记忆注入

### 功能设计

Agent 启动时，Sage 自动读取 `memory.auto_load` 列表中的文件，内容追加到 system_prompt。

格式：

```
<原始 system_prompt>

--- AGENT MEMORY ---
### MEMORY.md
<文件内容>

### CONVENTIONS.md
<文件内容>
--- END MEMORY ---
```

- 文件不存在 → 静默跳过
- 文件存在但为空 → 静默跳过
- Agent 在 VM 内通过 `write`/`edit` 工具更新记忆文件（正常工具调用，无特殊机制）

### 方案逻辑

`sage-runtime/system_prompt.rs` 已有 `SystemPromptBuilder::section()`，内存注入：

```rust
// sage-runtime/src/memory.rs — 新文件

pub async fn load_memory_block(
    auto_load: &[String],
    workspace_host: &Path,
) -> String {
    let mut parts = Vec::new();
    for file in auto_load {
        let path = workspace_host.join(file);
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            let content = content.trim();
            if !content.is_empty() {
                parts.push(format!("### {file}\n{content}"));
            }
        }
    }
    if parts.is_empty() {
        return String::new();
    }
    format!(
        "--- AGENT MEMORY ---\n{}\n--- END MEMORY ---",
        parts.join("\n\n")
    )
}

// sage-runtime/src/engine.rs 中，build_system_prompt 时

let memory_block = load_memory_block(
    &config.memory.auto_load,
    workspace_host,
).await;

let system_prompt = SystemPrompt::builder()
    .cacheable_section("base", &config.system_prompt)
    .section("memory", memory_block)   // 空字符串时 SystemPromptBuilder 自动跳过
    .build();
```

### To-Do

- [ ] `sage-runner/src/config.rs` — 加 `MemoryConfig` + `MemoryInjectMode`
- [ ] `sage-runtime/src/memory.rs` — 新文件，`load_memory_block()`
- [ ] `sage-runtime/src/engine.rs` — 组装 system_prompt 时调用 `load_memory_block`
  - workspace_host 路径从哪来：`SandboxConfig::workspace_host`，engine 启动参数传入
- [ ] 单元测试：无文件 / 有文件 / 部分为空 各场景

---

## M4：TUI（chat 模式）

### 功能设计

```
$ sage chat --agent feishu

Sage ✦ feishu
飞书专员 — 管理我的飞书工作空间
Workspace: ~/.sage/agents/feishu/workspace
────────────────────────────────────────

You > 帮我把明天 10 点的面试日程发给张三

● Thinking...
  我来帮你安排明天 10 点的面试日程。先看看你的飞书日历……

⚙ bash                                              ▸
  curl -X GET https://open.feishu.cn/...
  ✓ 200 OK

  好的，我已查到你的日历空闲时间。现在创建日程并发送邀请……

⚙ bash                                              ▸
  curl -X POST ...
  ✓ 201 Created

  ✓ 日程已创建并发送邀请给张三：
  - 时间：明天 10:00–11:00
  - 标题：[面试] 张三 - 后端工程师
  - 视频链接：已附加飞书会议链接

You > _
```

**交互规范**：
- `You >` 提示符，支持多行输入（`\` 续行 或 Enter 提交单行）
- LLM 输出流式打印
- 工具调用：`⚙ <tool_name>` 显示 + 可折叠命令摘要 + `✓`/`✗` 结果
- `Ctrl+C`：优雅退出（停止当前 turn，VM 清理，workspace 保留）
- `Ctrl+D` / `/exit` / `/quit`：退出会话
- `/clear`：清空 terminal，不清 context
- `/reset`：清空 context，开始新会话（workspace 不受影响）

**v1.0.0 TUI 实现方案**：不引入 ratatui，用 `rustyline`（readline）+ 标准 stdout 流式输出。ratatui 是 v2.0 的事。

### 方案逻辑

```
chat 主循环：

1. 解析 --agent <name> → 加载 AgentConfig + 确定 workspace_host
2. 打印 header（name/description/workspace）
3. 初始化 SageEngine（含 memory 注入）
4. 启动 Sandbox VM
5. loop {
     print("You > ")
     readline → user_input（Ctrl+D / /exit → break）
     stream = engine.run(user_input)
     while let Some(event) = stream.next().await {
       match event {
         AgentEvent::Text { delta } => print!("{delta}")
         AgentEvent::ToolStart { name, input } => print_tool_start(name, input)
         AgentEvent::ToolEnd { result } => print_tool_end(result)
         AgentEvent::AgentEnd { .. } => break
       }
     }
   }
6. VM 清理（sandbox.stop()）
```

多轮 context 保留：`SageEngine` 在 chat 模式下持有 conversation history，每轮 `run()` 追加上轮结果。

### To-Do

- [ ] `sage-cli/src/chat.rs` — 新文件
  - `pub async fn run_chat(agent_name: &str) -> Result<()>`
  - `fn print_header(config: &AgentConfig, workspace: &Path)`
  - `fn print_tool_start(name: &str, input: &Value)`
  - `fn print_tool_end(result: &str, success: bool)`
  - 主循环（rustyline + stream 消费）
- [ ] `sage-cli/Cargo.toml` — 加 `rustyline` 依赖
- [ ] `sage-runtime/src/engine.rs` — chat 模式下 context 累积
  - 当前 `run()` 是 stateless one-shot，chat 模式需要 `run_with_history(history, message)`
  - 或：`SageEngine` 持有 `conversation: Vec<Message>`，`chat()` 方法追加
- [ ] `sage-cli/src/main.rs` — 添加 `Chat { agent: String }` 子命令
- [ ] 手动测试：端到端 `sage chat --agent coding-assistant` 跑通

---

## M5：配套 CLI 命令补全

| 命令 | 说明 | 实现位置 |
|------|------|---------|
| `sage init --agent <name>` | 创建 Agent（wizard） | `src/init.rs` |
| `sage list` | 列出所有 Agent | `src/registry.rs` |
| `sage chat --agent <name>` | TUI 交互 | `src/chat.rs` |
| `sage run --agent <name> --message "..."` | One-shot（现有，扩展支持 `--agent`） | `src/serve.rs` |
| `sage validate --agent <name>` | 校验配置 | `src/registry.rs` |

### 现有 `run` 命令改造

当前 `--config` 路径接受 yaml 路径，扩展为：
- `--agent feishu` → 解析 `~/.sage/agents/feishu/agent.yaml`
- `--config ./configs/foo.yaml` → 直接路径（保持现有行为）

两者在 `resolve_agent()` 中统一处理。

### To-Do

- [ ] `sage-cli/src/main.rs` — 所有新子命令入口
- [ ] `sage-cli/src/registry.rs` — `resolve_agent` 支持 name 和 path 两种形式
- [ ] `sage-cli/src/serve.rs` — 现有 `run_local_test` 兼容 `--agent` 参数

---

## 执行顺序

```
M1（注册/发现）→ M2（workspace mount）→ M3（memory 注入）→ M4（TUI）→ M5（命令补全）
```

M1-M3 是基础，可以没有 TUI 先用 `run` 命令验证。M4 在 M1-M3 全绿后开始。

---

## 依赖变更

**sage-cli/Cargo.toml**（新增）：
```toml
rustyline = "14"   # readline for TUI input
```

**sage-runner/Cargo.toml**（无新增，复用已有 serde）

**sage-runtime/Cargo.toml**（无新增，复用 tokio::fs）
