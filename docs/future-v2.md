# Sage — 定位与规划

## 一句话

**领域专家 Agent 应用 — 给任意领域配置一个有独立工作空间、有记忆沉淀的专员。**

Sage 是一个 Agent 应用，不是 Agent 框架。你可以配置"飞书专员""GitHub Review 专员""信息调研专员"，每个专员有自己的持久化工作空间和知识积累，可以通过 TUI 交互，也可以被 Runeforge 无人值守调度。

---

## 定位

### 和 CC / Codex 的关系

Sage 不是 CC/Codex 的竞争者，是它们的补充：

| | CC / Codex | Sage |
|--|--|--|
| 性质 | 编码专家（固定领域） | 领域专家（可配置领域） |
| 使用方式 | TUI 交互 / CLI | TUI 交互 / Caster 调度 |
| 领域 | 软件开发 | 任意（飞书、GitHub、调研、…） |
| 工具来源 | 内置固定工具集 | YAML 声明 + 白名单 |
| 记忆 | 有限（CLAUDE.md） | 持久化工作空间 + 知识文件 |
| 沙箱 | 宿主文件系统 | microVM 硬件隔离 |
| 后台调度 | 不支持 | 支持（Caster 模式） |

**核心差异：Sage 的每个 Agent 是一个有独立工作空间和记忆的"专员"，不是无状态的 one-shot 执行器。**

### 在 rune-ecosystem 的位置

```
┌──────────────────────────────────────────────────┐
│  Runeforge  软件锻造平台                           │
│  Issue → Task → 分配给 CC / Codex / Sage Agent   │
└───────────────────────┬──────────────────────────┘
                        │ 通过 Rune 路由任务（v1.1+）
┌───────────────────────▼──────────────────────────┐
│  Rune Runtime  分布式调度层                        │
│  Caster 注册 / 任务路由 / 扩缩容                   │
└───────────────────────┬──────────────────────────┘
                        │ Sage 注册为 Caster（Caster 模式）
┌───────────────────────▼──────────────────────────┐
│  Sage  领域专家 Agent 应用                         │
│  TUI 交互 + 持久工作空间 + 记忆沉淀 + 沙箱隔离     │
└──────────────────────────────────────────────────┘
```

---

## Agent 的完整定义

一个 Sage Agent 由以下五个部分构成：

### 1. Identity（身份）

Agent 的名字和用途声明，决定目录命名和 TUI 显示。

### 2. Config（配置）

YAML 文件声明的静态配置：LLM、系统提示词、工具策略、沙箱参数、约束。

### 3. Workspace（持久化工作空间）

主机上的一个目录，每次启动时 mount 进 microVM 的 `/workspace`。Agent 在这里写代码、执行脚本、保存文件——关闭 VM 后内容持久留在主机。

### 4. Memory（知识/记忆）

工作空间内的特殊文件，Sage 在 Agent 启动时自动注入为上下文。Agent 通过 write/edit 工具主动更新这些文件，在一次次交互中沉淀知识。

### 5. Runtime（执行环境）

每次会话在一个新的 microVM 实例中执行，工作空间 mount 进来。会话结束后 VM 销毁，工作空间保留。

---

## 全局目录结构

```
~/.sage/
├── config.toml              # Sage 全局配置（默认 LLM、日志级别等）
└── agents/
    ├── feishu/
    │   ├── agent.yaml       # Agent 配置（唯一声明文件）
    │   └── workspace/       # 持久化工作空间（mount 到 VM 的 /workspace）
    │       ├── MEMORY.md        # Agent 积累的知识（自动注入）
    │       ├── CONVENTIONS.md   # 用户使用规范（自动注入）
    │       ├── scripts/         # Agent 写的可复用脚本
    │       └── ...
    ├── github-review/
    │   ├── agent.yaml
    │   └── workspace/
    └── research/
        ├── agent.yaml
        └── workspace/
```

---

## Agent YAML 完整格式

```yaml
# ~/.sage/agents/feishu/agent.yaml

name: feishu
description: "飞书专员 — 管理我的飞书工作空间：文档、日程、消息"
version: "1.0.0"

# LLM 配置
llm:
  provider: anthropic
  model: claude-sonnet-4-6
  max_tokens: 8192
  # temperature: 1.0  # 可选，默认由 provider 决定

# 系统提示词
# Sage 自动在此之后追加 memory 文件内容
system_prompt: |
  你是我的飞书专员。你有一个独立的工作空间 /workspace，可以在里面写代码、
  执行脚本、保存文件和积累知识。

  每次执行任务时，请先确认任务范围，逐步执行，完成后给出简明总结。
  如果过程中发现了我的偏好或规范（例如文档命名方式、文件夹结构），
  主动更新 /workspace/MEMORY.md 或 /workspace/CONVENTIONS.md。

# 工具策略
tools:
  bash:
    enabled: true
    allowed_binaries:
      - python3
      - node
      - curl
      - jq
      - git
      - rg
      - fd
  read:
    enabled: true
    allowed_paths: ["/workspace"]
  write:
    enabled: true
    allowed_paths: ["/workspace"]
  edit:
    enabled: true
    allowed_paths: ["/workspace"]
  grep:
    enabled: true
  find:
    enabled: true
  ls:
    enabled: true

# 沙箱配置
sandbox:
  cpus: 2
  memory_mib: 2048
  network: full        # full | airgapped | whitelist（whitelist 未来实现）

# 约束
constraints:
  max_turns: 50
  timeout_secs: 600

# 记忆/知识配置
memory:
  # 启动时自动读取并注入到上下文的文件列表（相对于 workspace 根目录）
  # 不存在的文件会被跳过（首次启动正常）
  auto_load:
    - MEMORY.md
    - CONVENTIONS.md
  # 注入方式：prepend_system（追加到 system_prompt 之后）
  # 或 initial_message（作为第一条 user/assistant 消息对注入）
  inject_as: prepend_system
```

---

## 记忆/知识系统

### 工作原理

```
Agent 启动
    │
    ▼
Sage 扫描 workspace/MEMORY.md、workspace/CONVENTIONS.md
    │
    ▼
文件内容拼接后追加到 system_prompt
（格式：--- MEMORY ---\n<内容>\n--- END MEMORY ---）
    │
    ▼
Agent Loop 开始，Agent 天然"知道"历史积累的内容
    │
    ▼
执行任务过程中，Agent 可用 write/edit 工具更新记忆文件
（无需人工干预，Agent 自行决定什么值得记录）
    │
    ▼
会话结束，VM 销毁，记忆文件留在主机 workspace
    │
    ▼
下次启动，新知识自动注入
```

### 记忆文件规范

每个文件有独立用途，Agent 在系统提示词中被告知各文件的语义：

| 文件 | 用途 | 更新时机 |
|------|------|---------|
| `MEMORY.md` | 积累的事实性知识（API 路径、频道 ID、常用文件位置） | 每次学到新的具体信息 |
| `CONVENTIONS.md` | 用户的偏好和规范（命名风格、文档格式、工作习惯） | 发现用户有明确偏好时 |

Agent 可自由在 workspace 中创建其他文件（脚本、草稿、参考资料），Sage 不强制管理这些文件。

### 记忆文件示例

```markdown
<!-- /workspace/MEMORY.md -->

# 飞书知识库

## 常用资源
- 面试日程文档 ID: `doccnXXXXXX`（面试安排汇总）
- 技术周会频道 ID: `oc_XXXXXX`
- 工程群频道 ID: `oc_YYYYYY`

## API 备注
- 创建文档事件需要等待 500ms 再查询，否则返回 404
- 批量更新日历时，单次最多 50 条

## 文件位置
- 招聘相关文档统一放在"人才发展"空间下的"招聘"文件夹
```

```markdown
<!-- /workspace/CONVENTIONS.md -->

# 用户偏好与规范

## 日历事件
- 面试事件命名：`[面试] 候选人姓名 - 岗位` （例：`[面试] 张三 - 后端工程师`）
- 时长默认 1 小时，除非明确说明
- 必须加上视频会议链接

## 文档格式
- 标题用中文
- 正文用中文，代码示例可用英文
- 重要内容用加粗，不用下划线
```

---

## 运行模式

### 模式 1：TUI 交互（v1.0.0）

```bash
# 启动飞书专员，进入交互对话
sage chat --agent feishu

# 或使用完整路径
sage chat --config ~/.sage/agents/feishu/agent.yaml
```

TUI 行为：
- 流式显示 LLM 输出
- 工具调用实时显示（调用名 + 参数摘要 + 结果状态）
- 多轮对话（会话内 context 保留）
- Ctrl+C 优雅退出，workspace 文件保留

### 模式 2：One-shot 执行

```bash
sage run --agent feishu --message "帮我把明天 10 点的面试日程发给张三"
```

### 模式 3：Caster 调度（v1.1+）

```bash
# 注册为 Rune Caster，接受 Runeforge 任务派发
sage serve --runtime localhost:50070
```

Runeforge 创建 issue 时指定 `execution_engine: sage, agent: feishu`，Sage 无人值守执行，结果推送回 Runeforge。

---

## v1.0.0 Scope：TUI 交互全流程跑通

**目标**：`sage chat --agent feishu` 能完整跑通一次多轮交互，从启动到会话结束，记忆自动注入和更新。

**不包含**：Caster 模式（v1.1）、Runeforge 集成（v1.1）、TUI 高级功能（历史回溯、多 Agent 切换）。

### M1：Agent 注册与发现

- [ ] `~/.sage/agents/<name>/agent.yaml` 规范解析
- [ ] `sage list` 列出所有已配置 Agent
- [ ] `sage init --agent <name>` 创建 Agent 模板（agent.yaml + workspace/）
- [ ] `sage validate --agent <name>` 校验配置合法性

### M2：Workspace 挂载

- [ ] `sandbox.workspace_host` 字段解析（`~/.sage/agents/<name>/workspace`）
- [ ] SandboxBuilder 支持将主机目录 mount 到 VM 的 `/workspace`
- [ ] 首次启动时自动创建 workspace 目录
- [ ] 会话结束后 workspace 内容保留验证

### M3：记忆注入

- [ ] 启动时扫描 `memory.auto_load` 列表
- [ ] 存在的文件读取内容，拼接成 memory block
- [ ] 注入到 system_prompt 末尾（`inject_as: prepend_system`）
- [ ] 不存在的文件静默跳过（首次启动正常）
- [ ] Agent 通过 write/edit 工具更新记忆文件（无需额外机制）

### M4：TUI

- [ ] `sage chat --agent <name>` 命令
- [ ] 交互式输入（readline / ratatui 简单输入行）
- [ ] 流式输出 LLM 文本
- [ ] 工具调用显示：`[tool: bash] ls /workspace` → `✓`
- [ ] 多轮对话（context 在会话内累积）
- [ ] Ctrl+C 优雅退出（VM 清理，workspace 保留）
- [ ] 简单错误提示（LLM 错误、沙箱崩溃）

### M5：核心工具（沙箱内执行）

- [ ] bash（白名单二进制）
- [ ] read / write / edit / grep / find / ls
- [ ] 所有工具路径限制在 `/workspace`（写操作）

### M5：配套命令

```bash
sage init --agent feishu        # 创建 Agent（生成 agent.yaml + workspace/）
sage list                       # 列出所有 Agent
sage chat --agent feishu        # 进入 TUI 交互
sage run --agent feishu \       # One-shot 执行
  --message "..."
sage validate --agent feishu    # 校验配置
```

---

## v1.1 Scope：Runeforge 集成（Caster 模式）

- `sage serve --runtime localhost:50070` 注册为 Rune Caster
- 接收 `{ agent: "feishu", message: "..." }` 任务
- 无 TUI，结果通过 Rune StreamSender 推送
- Runeforge Issue 支持 `execution_engine: sage, agent: <name>`

---

## 架构（保持不变）

### Crate 结构

```
sage/
├── crates/
│   ├── sage-cli/          # CLI 入口：sage chat / run / serve / init / list
│   ├── sage-runtime/      # Agent 内核：Loop + LLM + Tools + Memory 注入
│   ├── sage-sandbox/      # 沙箱引擎：msb_krun + Workspace mount
│   ├── sage-protocol/     # Host↔Guest 线协议（CBOR）
│   ├── sage-guest/        # VM 内 PID 1
│   └── sage-runner/       # AgentConfig YAML 解析 + ToolPolicy
```

### 新增 sage-runner 字段（v1.0.0）

```rust
// crates/sage-runner/src/config.rs

pub struct AgentConfig {
    pub name: String,
    pub description: String,
    pub version: String,
    pub llm: LlmConfig,
    pub system_prompt: String,
    pub tools: ToolsConfig,
    pub sandbox: SandboxConfig,
    pub constraints: ConstraintsConfig,
    pub memory: MemoryConfig,      // 新增
}

pub struct MemoryConfig {
    pub auto_load: Vec<String>,         // 相对 workspace 的文件路径列表
    pub inject_as: MemoryInjectMode,    // PrependSystem | InitialMessage
}

pub enum MemoryInjectMode {
    PrependSystem,    // 追加到 system_prompt 末尾
    InitialMessage,   // 作为首条 user/assistant 消息注入
}

pub struct SandboxConfig {
    pub cpus: u32,
    pub memory_mib: u32,
    pub network: NetworkMode,
    pub workspace_host: Option<PathBuf>,  // 新增：主机工作空间路径
}
```

### 记忆注入流程（sage-runtime）

```rust
// SageEngine::builder() 内部

async fn build_system_prompt(config: &AgentConfig, workspace: &Path) -> String {
    let mut prompt = config.system_prompt.clone();

    let memory_blocks = load_memory_files(&config.memory.auto_load, workspace).await;
    if !memory_blocks.is_empty() {
        prompt.push_str("\n\n--- AGENT MEMORY ---\n");
        prompt.push_str(&memory_blocks.join("\n\n"));
        prompt.push_str("\n--- END MEMORY ---");
    }

    prompt
}

async fn load_memory_files(files: &[String], workspace: &Path) -> Vec<String> {
    let mut blocks = Vec::new();
    for file in files {
        let path = workspace.join(file);
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            if !content.trim().is_empty() {
                blocks.push(format!("### {file}\n{content}"));
            }
        }
        // 文件不存在时静默跳过
    }
    blocks
}
```

---

## Git 工作流

- `dev` — 日常开发
- `main` — 发版
- Conventional Commits: `feat(scope): description`
- scope: `cli`, `sandbox`, `protocol`, `guest`, `runner`, `runtime`
