# 路线图（v0.7–v0.9）— Sage 独立 Agent 产品演进

> v0.7–v0.9 将 Sage 从"可嵌入执行引擎"演进为"领域专家 Agent 独立产品"，对齐 CC/Codex 产品形态。
> 完整产品定位见 `docs/future.md`，Hook/Skill 研究报告见 `docs/research-cc-hooks-skills.md`。

---

### v0.7.0 — Agent Registry + Workspace + Memory

**目标**：建立 Sage Agent 身份模型。Agent 有名字、有独立持久化工作空间、有跨会话记忆。

**背景**：v0.0.1 是无状态 one-shot 执行器，每次运行需要显式传入配置文件。v0.7.0 引入 Agent 注册中心：Agent 有固定的 `~/.sage/agents/<name>/` 主目录，配置与工作空间分离，工作空间 mount 进 microVM，记忆文件在启动时自动注入 system_prompt。

#### M1：Agent 注册与发现

- [ ] `~/.sage/agents/<name>/agent.yaml` 规范解析（新增 `memory` 字段、`sandbox.workspace_host`）
- [ ] `sage init --agent <name>` — 创建 Agent 目录结构 + 模板 agent.yaml + 空白 workspace/
- [ ] `sage list` — 列出所有已注册 Agent（name + description + workspace 大小）
- [ ] `sage validate --agent <name>` — 校验 agent.yaml 合法性、workspace 路径存在性

```
~/.sage/agents/<name>/
├── agent.yaml         # 静态配置（LLM、工具、沙箱、记忆策略）
└── workspace/         # 持久化工作空间（mount 到 VM /workspace）
    ├── SCHEMA.md          # Wiki 锚点（wiki 根路径、frontmatter 规范）
    ├── AGENT.md           # Schema 层：Agent 认知框架（自动注入 system prompt）
    ├── memory/            # 操作性短期记忆
    │   ├── MEMORY.md      #   索引文件（自动注入 system prompt）
    │   ├── user.md        #   用户画像
    │   └── context-*.md   #   当前项目上下文
    ├── raw/               # 不可变原始记录
    │   └── sessions/      #   对话归档（ingest 后标记 processed）
    ├── wiki/              # LLM 维护的结构化知识库
    │   ├── index.md       #   页面目录索引（按类别分组）
    │   ├── log.md         #   追加式维护日志
    │   ├── overview.md    #   全局综合理解
    │   └── pages/         #   所有 wiki 页面（flat，slug 命名）
    ├── assets/            # 附件（图片、PDF 等）
    ├── craft/             # Agent 自管理的可复用产物（SOP / 脚本 / 模板 / 素材）
    │   ├── <craft-name>/  #   SOP 类：CRAFT.md + metrics.json
    │   ├── <script>.py    #   脚本类
    │   └── <template>.md  #   模板类
    └── metrics/           # 任务度量记录（TaskRecord）
        ├── <task_id>.json #   每次 UserDriven 任务一条，ULID 命名
        └── summary.json   #   最近 N 条任务的滚动聚合
```

#### M2：Workspace 挂载

- [ ] `AgentConfig.sandbox.workspace_host: Option<PathBuf>` — 默认 `~/.sage/agents/<name>/workspace`
- [ ] tilde 展开：`~/...` 在 config 解析时转为绝对路径
- [ ] `SandboxBuilder` 将 workspace_host 作为读写 `VolumeMount` 传入，guest path = `/workspace`
- [ ] 首次启动时自动创建 workspace 目录（`fs::create_dir_all`）
- [ ] 会话结束 VM 销毁后，workspace 内容保留验证

#### M3：知识系统初始化

- [ ] `AgentConfig.memory.auto_load: Vec<String>` — 相对 workspace 根目录的文件列表（默认 `["AGENT.md", "memory/MEMORY.md"]`）
- [ ] `AgentConfig.memory.inject_as: MemoryInjectMode` — `PrependSystem`（追加到 system_prompt 末尾）或 `InitialMessage`
- [ ] 启动时读取 auto_load 文件，内容非空则拼成 memory block（`--- AGENT MEMORY ---` 格式）
- [ ] 通过 `SystemPromptBuilder.cacheable_section("memory", ...)` 注入，享受 prompt caching
- [ ] 文件不存在时静默跳过（首次启动 workspace 为空，正常）
- [ ] 首次启动时初始化 workspace 目录结构：
  - `memory/`、`raw/sessions/`、`wiki/pages/`、`assets/`、`craft/`、`metrics/`
  - 生成 `SCHEMA.md`（写入 wiki 根路径、frontmatter 规范、cross-reference 约定）
  - 生成 `wiki/index.md`、`wiki/log.md`、`wiki/overview.md` 空模板
- [ ] Skill 扫描（两层）：
  - `.agent/skills/`（平台级，自动安装 `chasey-myagi/llm-wiki` 的 4 个 Skills，只读）
  - `workspace/craft/`（Agent 自管理，可读写，扫描 `CRAFT.md` 的 SOP 类 + 脚本/模板索引）
  - SOP 类 Craft frontmatter 注入 system prompt（name + description + score）
- [ ] DaemonLoop 会话类型标记：`UserDriven` / `WikiMaintenance` / `CraftEvaluation`
- [ ] **TaskRecord 采集（MetricsCollector）**：
  - 订阅 AgentEvent 流（零侵入 agent_loop），累积每轮 `TurnEnd.message.usage`、工具调用/错误次数、压缩次数
  - UserDriven 会话结束时写 `workspace/metrics/<task_id>.json`（ULID 命名）
  - 定期更新 `workspace/metrics/summary.json`（最近 50 条任务的滚动聚合）
  - WikiMaintenance / CraftEvaluation 会话不写 TaskRecord（避免污染用户任务数据）

新增 `sage-runner` 结构体：

```rust
pub struct MemoryConfig {
    pub auto_load: Vec<String>,
    pub inject_as: MemoryInjectMode,
}

pub enum MemoryInjectMode { PrependSystem, InitialMessage }

pub struct SandboxConfig {
    pub cpus: u32,
    pub memory_mib: u32,
    pub network: NetworkMode,
    pub workspace_host: Option<PathBuf>,
}

/// Wiki 维护触发策略（轻量配置，无需 WikiConfig 结构体）
/// 在 agent.yaml 中配置：
///   wiki:
///     enabled: true
///     cooldown_secs: 300       # 两次 wiki 维护之间的最小间隔
///     trigger_sessions: 3      # 积累多少未处理 session 后触发

/// 会话类型标记（防止 wiki 维护触发新的 wiki 维护）
pub enum SessionType {
    UserDriven,        // 正常用户对话 → 结束后检查是否需要 wiki 维护
    WikiMaintenance,   // wiki 整理 → 不触发新的 wiki 维护
    CraftEvaluation,   // skill 评估/创建 → 不触发新的 wiki 维护
}

/// 每次任务的行为快照（MetricsCollector 订阅 AgentEvent 流零侵入采集）
/// 写入 workspace/metrics/<task_id>.json，仅 UserDriven 会话写入
pub struct TaskRecord {
    pub task_id: String,           // ULID
    pub agent_name: String,
    pub model: String,
    pub config_hash: String,       // sha256(agent.yaml)

    pub started_at: u64,
    pub ended_at: u64,
    pub duration_ms: u64,

    // token 消耗（TurnEnd.message.usage 逐轮累加）
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,

    // 行为计数
    pub turn_count: u32,
    pub tool_call_count: u32,
    pub tool_error_count: u32,
    pub compaction_count: u32,

    // 结果
    pub success: bool,
    pub failure_reason: Option<String>,

    // 知识层（Phase 2+）
    pub crafts_active: Vec<String>,  // 本次注入了哪些 Craft（Phase 2+）
}
```

**验收标准**：

```bash
sage init --agent feishu        # 创建 ~/.sage/agents/feishu/
sage list                       # feishu  [飞书专员 — ...]  workspace: 0B
sage validate --agent feishu    # ✓ OK
# 在 workspace/MEMORY.md 写入内容，运行任务后确认内容注入到 system_prompt
```

---

### v0.8.0 — TUI + Standard Rootfs + Session Persistence

**目标**：`sage chat --agent feishu` 能完整跑通多轮交互，流式输出，工具调用可视化，会话上下文支持恢复。

#### M4：TUI 交互（sage chat）

- [ ] `sage chat --agent <name>` CLI 命令（带 `--resume` flag 支持恢复）
- [ ] 交互式输入行（rustyline 或 crossterm 简单输入，无需完整 ratatui）
- [ ] 流式输出 LLM 文本（逐 token 写到 stdout）
- [ ] 工具调用可视化：`[tool: bash] ls /workspace` → 结束后追加 `✓ (0.3s)` 或 `✗ (错误摘要)`
- [ ] 多轮对话：会话内 context 累积，Agent 记住上文
- [ ] 内置命令：`/exit`（优雅退出）、`/reset`（清空本轮 context，workspace 保留）
- [ ] Ctrl+C 优雅退出：发 Stop 信号给 engine，等 VM 清理完成后退出
- [ ] 错误提示：LLM API 错误、沙箱崩溃、工具超时

#### 沙箱模式：microvm + none

两种沙箱模式，覆盖生产和本地开发两种场景：

| 模式 | 适用 | 工具来源 | 安全性 |
|------|------|---------|--------|
| `none`（本地默认） | 本地部署 / 可信环境 / 早期用户 | 宿主机 PATH（brew / npm / pip 全部可用） | 无沙箱，类 Claude Code |
| `microvm`（生产升级） | 服务器 / 多租户 / 不可信任务 | rootfs + tools/ 持久化卷（Linux 二进制） | KVM/HVF 硬件虚拟化隔离 |

```yaml
# 生产配置（默认）
sandbox:
  mode: microvm
  workspace_size_gb: 5
  network: whitelist
  allowed_hosts: [api.feishu.cn]

# 本地开发配置（agent.yaml.dev 或 --dev flag 覆盖）
sandbox:
  mode: none   # 宿主机直接执行，feishu / gh / npm 等工具立即可用
```

- [ ] `SandboxConfig.mode: SandboxMode` 枚举（`Microvm | None`）
- [ ] `mode: none` 下 bash tool 直接 `std::process::Command::spawn()`，不经过 VM
- [ ] `mode: none` 下 `allowed_binaries` 仍然生效（最小化权限，即使无 VM）
- [ ] `sage chat --dev` / `sage run --dev` flag：强制 `mode: none`，覆盖 agent.yaml 中的 mode

#### Standard Rootfs Bundle（microvm 模式）

内置分级 rootfs 包，降低 microvm 模式的工具供应复杂度：

| Tier | 包含 | 适用 |
|------|------|------|
| `minimal` | sage-guest + busybox | 安全敏感场景，最小攻击面 |
| `standard` | minimal + curl / python3 / jq / git / rg / fd / gh | 默认，覆盖大多数 Agent |
| `custom` | `rootfs_path` 指定预构建 tar.gz | 需要特定 SDK 的 Agent（少数场景） |

**生产工具供应策略（microvm 模式）**：优先用 `standard` rootfs 内已有的工具，其次用 `tools/` 持久化卷。`custom` rootfs 是最后手段。

**tools/ 持久化卷**：`~/.sage/agents/<name>/tools/` 随 workspace 一同 mount 进 VM（路径 `/agent-tools/`），SessionStart hook 首次运行时安装 Linux 版依赖，后续 cold start 直接挂载不重复安装。

```yaml
# agent.yaml（生产，feishu Agent）
sandbox:
  mode: microvm
  network: whitelist
  allowed_hosts: [api.feishu.cn]
  # tools/ 目录自动挂载，无需配置

# hooks/session-start.sh（首次安装 feishu SDK 到 tools/ 卷）
# if [ ! -f /agent-tools/lib/python3/lark_oapi ]; then
#   pip3 install --target=/agent-tools/lib/python3 lark-oapi
# fi
```

- [ ] `SandboxConfig.rootfs: RootfsTier` 字段解析（minimal / standard / custom）
- [ ] standard tier：构建时将工具链静态编译打包进 rootfs
- [ ] custom tier：解析 `rootfs_path`，挂载用户提供的 rootfs tar.gz
- [ ] tools/ 卷自动挂载：`~/.sage/agents/<name>/tools/` → VM 内 `/agent-tools/`（读写，随 workspace 一起持久化）
- [ ] `$PATH` 在 VM 内预置 `/agent-tools/bin`，让 tools/ 卷中的二进制直接可调用

#### Daemon 模式

Agent 作为常驻进程运行，TUI 只是接入点，VM 和对话历史持续存活。

- [ ] `sage start --agent <name>` — 启动 daemon（VM 启动、Memory 注入、Agent 进入待命状态）
- [ ] daemon 监听 Unix socket：`/tmp/sage-<name>.sock`（供 connect/send 接入）
- [ ] `sage connect --agent <name>` — 接入 TUI（attach 到 socket，流式 I/O）
- [ ] `sage disconnect` / Ctrl+D — 断开 TUI（daemon 继续运行，Agent 继续待命）
- [ ] `sage stop --agent <name>` — 关闭 daemon（优雅退出 Agent Loop，VM 销毁）
- [ ] `sage send --agent <name> "<message>"` — 非交互式发消息，等结果，适合脚本调用
- [ ] `sage status` — 列出所有运行中的 daemon（name + PID + 运行时长 + 对话轮数）
- [ ] daemon 崩溃后自动写 crash log，workspace 内容不受影响

**知识蒸馏（PreCompact hook 触发）**：

- [ ] PreCompact event 触发时，hook 提示 Agent：先更新 memory/MEMORY.md，再压缩
- [ ] 系统提示词中告知 Agent：有值得记住的信息，随时主动写入 `/workspace/memory/`
- [ ] compact 后：Memory 仍然在 system prompt 里（每次 compact 重新注入），对话历史被压缩

**Wiki 自动维护（IDLE hook 触发）**：

- [ ] UserDriven 会话结束后，对话记录归档到 `raw/sessions/`（标记 unprocessed）
- [ ] PostProcessing hook 检查未处理 session 数量 ≥ `wiki.max_unprocessed_sessions`
- [ ] 触发 WikiMaintenance 会话：Agent 使用 wiki_* 工具从 raw sessions 中提炼知识
- [ ] WikiMaintenance 期间用户新消息进入优先级队列，维护完成后立即处理
- [ ] wiki 维护冷却期：两次维护之间至少间隔 `wiki.maintenance_cooldown_secs`

**验收标准**：

```bash
# 启动 daemon
sage start --agent feishu
# [daemon] feishu 已启动 (PID 12345)，workspace: ~/.sage/agents/feishu/workspace/

# 接入 TUI 对话
sage connect --agent feishu
# > 把面试时长改为 45 分钟
# < 好的，已记录到 MEMORY.md [tool: write] ✓
# [Ctrl+D 断开]

# daemon 还在运行
sage status
# feishu  PID:12345  运行 2h15m  12 轮对话

# 再次接入，对话历史还在
sage connect --agent feishu
# < [已接入 feishu，12 轮对话历史]
# > 帮我创建明天的面试（45 分钟规则自动生效）

# 停止
sage stop --agent feishu
```

---

### v0.9.0 — Hook & Skill 系统

**目标**：让 Agent 具备可扩展的生命周期钩子和能力模块，向 Claude Code 的 Hook/Skill 机制对齐。

**背景**：CC 有 27 事件 Hook 系统 + Markdown Skill 系统，Sage 提取适合单用户领域专家产品的精简版本。完整分析见 `docs/research-cc-hooks-skills.md`。

#### H1：Hook 系统（12 核心事件）

从 CC 的 27 个事件中精选，去掉多 Agent 协作类（SubagentStart/Stop、TeammateIdle、TaskCreated 等），增加 3 个 Sage 特有事件：

| Event | 优先级 | 用途 |
|-------|-------|------|
| `SessionStart` | P0 | 初始化环境、注入动态上下文（替代硬编码启动逻辑） |
| `UserPromptSubmit` | P0 | 拦截/增强用户输入（最常用） |
| `PreToolUse` | P0 | 工具调用前审计/安全防护（exit 2 = 阻断） |
| `PostToolUse` | P0 | 工具调用后回调/副作用触发 |
| `Stop` | P0 | Agent 结束前质量检查，exit 2 = 要求继续 |
| `SessionEnd` | P1 | 保存状态、清理外部资源 |
| `PreCompact` | P1 | 压缩前自定义处理（exit 2 = 阻断压缩） |
| `PostCompact` | P1 | 压缩后通知/记录 |
| `FileChanged` | P1 | 监听 workspace 文件变化（替代轮询） |
| `MemoryUpdated` *(Sage)* | P2 | MEMORY.md 写入后触发（知识沉淀回调） |
| `AgentSwitched` *(Sage)* | P2 | `/switch <agent>` 切换 Agent 时 |
| `WorkspaceChanged` *(Sage)* | P2 | 工作目录变更 |

**4 种 Hook 类型（与 CC 一致）**：

| 类型 | 执行方式 | 适用场景 |
|------|---------|---------|
| `command` | 宿主 shell 执行脚本（stdin JSON + exit code） | 80% — 审计/日志/拦截/注入 |
| `http` | POST JSON 到 webhook URL | 外部系统集成、Slack/飞书通知 |
| `prompt` | 调小模型语义评估 | 无法用 regex 判断的内容 |
| `agent` | 启动子 Sage 实例验证 | 需要工具调用的复杂验证 |

**Exit code 协议**（command hook）：
- `exit 0` — 成功，stdout 按事件语义决定是否注入模型
- `exit 2` — 阻断，stderr 注入模型，让模型感知被阻断的原因
- `exit N` — 用户可见错误（stderr 展示给用户），执行继续

**Hook 配置位置**：

```yaml
# ~/.sage/agents/feishu/agent.yaml（Agent 级）
hooks:
  SessionStart:
    - hooks:
        - type: command
          command: "python3 ~/.sage/agents/feishu/init.py"
  PreToolUse:
    - matcher: "bash"        # 只在 bash 工具时触发
      hooks:
        - type: command
          command: "bash ~/.sage/agents/feishu/hooks/audit.sh"
          if: "bash(curl *)" # 只在 curl 子命令时触发

# ~/.sage/hooks.yaml（全局，跨所有 Agent）
PreToolUse:
  - matcher: "bash"
    hooks:
      - type: command
        command: "~/.sage/global-hooks/security-audit.sh"
```

**实现任务**：

- [ ] `HookEvent` 枚举（12 个事件）+ tokio broadcast channel 广播系统
- [ ] command hook 执行引擎：fork shell 进程，写 stdin JSON，读 exit code + stdout/stderr
- [ ] exit 2 协议：阻断工具调用，将 stderr 作为 steering message 注入 agent loop
- [ ] `agent.yaml` 中 `hooks` 字段解析 + 全局 `~/.sage/hooks.yaml` 合并加载
- [ ] Matcher 过滤（tool name 前缀匹配）+ `if` 条件（权限规则语法子集）
- [ ] http hook：reqwest POST，body = stdin JSON 格式的 payload
- [ ] prompt/agent hook：预留接口，v0.9.x 实现

#### S1：Skill + Craft 系统（两级架构）

**全局 Skills**（`~/.sage/skills/`）= Markdown + YAML frontmatter，`/skill-name` 调用时完整注入当前对话。只读，开发者维护。

**Craft**（`workspace/craft/`）= Agent 自管理的可复用产物，类型任意（SOP、脚本、模板、素材）。通过 `craft_manage` 工具创建/编辑/评估。

**目录结构**：

```
~/.sage/
├── skills/                      # 全局 Skills（只读 Markdown）
│   ├── research-template.md
│   └── batch-update.md
└── agents/
    └── feishu/
        ├── agent.yaml
        └── workspace/
            └── craft/           # Agent Craft（可读写，类型任意）
                ├── batch-calendar-update/   # SOP 类
                │   ├── CRAFT.md             # Craft 正文
                │   └── metrics.json         # token 效率评分
                ├── weekly-report.py         # 脚本类
                └── summary-template.md      # 模板类
```

**SOP 类 Craft 文件格式（CRAFT.md，兼容 CC frontmatter 结构）**：

```markdown
---
name: feishu-schedule
description: "批量更新飞书日历"
type: sop                        # sop / script / template / asset
whenToUse: "用户要批量操作多个日历事件时"
allowedTools: [bash, read, write]
agent: feishu
version: 3
---

# 批量日历更新

步骤：
1. 读取 /workspace/schedule.csv 中的事件列表
2. 逐条调用飞书日历 API 更新...
```

**Craft 生命周期**：

```
触发条件：
  - 复杂任务成功完成（5+ 工具调用）
  - 从错误中恢复的有效路径
  - 反复出现的同类任务模式（3+ 次）

创建 → craft_manage(action: "create", name, type, content)
评估 → SOP 类每次使用记录 token 消耗到 metrics.json
优化 → 低效 SOP Craft 触发 CraftEvaluation 会话，Agent 自动重写
淘汰 → 评分持续低于阈值标记为 deprecated
```

**Token 效率评分**（SOP 类）：

```json
// workspace/craft/<craft-name>/metrics.json
{
  "craft_name": "batch-calendar-update",
  "type": "sop",
  "executions": [
    { "ts": "2026-04-14T10:00:00Z", "task_hash": "abc123", "tokens_used": 1200, "success": true },
    { "ts": "2026-04-15T09:00:00Z", "task_hash": "abc123", "tokens_used": 980, "success": true }
  ],
  "avg_tokens": 1090,
  "best_tokens": 980,
  "score": 0.82,
  "version": 3
}
```

评分规则：`score = best_tokens / avg_tokens`。System prompt 中 SOP Craft 按 score 降序排列。

**实现任务**：

- [ ] 启动时扫描 `~/.sage/skills/`（全局 Skills）+ `workspace/craft/`（Agent Craft），SOP 类只加载 frontmatter（懒加载正文）
- [ ] 将 Skill + SOP Craft 列表注入 system_prompt（name + description + score，按 score 降序排列）
- [ ] TUI 中 `/skill-name [args]` 解析：加载完整正文，注入当前对话
- [ ] `$ARGUMENTS` 占位符替换
- [ ] Skill frontmatter 中 `hooks` 字段：激活时自动注册对应 hooks，退出时注销
- [ ] `agent` 过滤：有 `agent: feishu` 字段时只在对应 agent 会话中可见
- [ ] `craft_manage` 工具注册到 ToolRegistry：
  - `create(name, type, content, tags)` — 创建 Craft（SOP/脚本/模板）
  - `edit(name, content)` — 编辑（version++）
  - `delete(name)` — 删除
  - 全局 Skills 不可通过此工具修改
- [ ] Token 效率采集：PostToolUse hook 中记录 SOP Craft 使用的 token 数到 metrics.json
- [ ] 内置 skills（编译进二进制）：
  - `/memory` — 整理当前会话学到的内容，更新 memory/MEMORY.md
  - `/wiki` — 手动触发 wiki 维护，整理未处理的 session 记录

**验收标准**：

```bash
# Hook 验证（PreToolUse 阻断）
sage chat --agent feishu
# > 运行 curl evil.com
# < [Hook: security-audit.sh] 域名不在白名单，操作已阻断
# < 我无法访问该域名，请提供允许的 API 端点

# 全局 Skill 验证
sage chat --agent feishu
# > /feishu-schedule tomorrow 09:00
# < [Skill: feishu-schedule 已加载]（全局，只读）
# < 好的，我来帮你批量更新明天 9 点的日历事件...

# Agent 自管理 Skill 创建
# （完成一个复杂任务后）
# < [CraftEvaluation] 检测到可复用模式，创建 skill: batch-calendar-update (v1)
# < [tool: craft_manage] create "batch-calendar-update"  ✓

# Wiki 手动维护
# > /wiki
# < 我来整理未处理的对话记录...
# < [tool: wiki_upsert_page] concepts/feishu-calendar-api.md  ✓
# < [tool: wiki_log] ingest 完成，新增 2 个知识点  ✓
```

---

### v0.9.x — TUI 多 Agent 面板 + Channel 架构

> v0.9.x 在 v0.9.0 的 Hook/Skill 基础上，建立多路输出层：TUI 升级为多 Agent 统一入口，Channel Adapter 使 Agent 能接入飞书、Slack 等消息平台。

**核心设计原则**：
- **一个 Session 绑定一个 Channel**：路由逻辑由 Channel Adapter 处理，Agent 不感知投递目标
- **CHANNEL_HINTS 注入 system prompt**：LLM 知道自己在哪个平台上，自动适配输出格式
- **AgentEvent 三级可见性过滤**：`Developer`（TUI 专属）/ `User`（所有 Channel）/ `Internal`（日志系统）

#### C1：AgentEvent 可见性系统

- [ ] `AgentEvent` 增加 `visibility: Visibility` 字段（枚举：`Developer | User | Internal`）
- [ ] 现有事件分类：
  - `Developer`：`ThinkingDelta`、`ToolExecutionStart/End`（含耗时）、`ContextStats`（token 用量）、原始 SSE stream
  - `User`：`TextDelta`（最终回复）、`AgentEnd`（任务完成）、`AgentError`
  - `Internal`：压缩事件、metrics、链路 span
- [ ] `EventStream` 支持按 `Visibility` 过滤订阅：`stream.subscribe(Visibility::User)`
- [ ] 事件广播通过 tokio broadcast channel，多 subscriber 独立消费

#### C2：TUI 多 Agent 面板

**目标**：`sage connect` 不再只接入单个 Agent，而是打开统一多 Agent 管理面板。

- [ ] 左侧 Agent 列表面板：显示所有 daemon 状态（IDLE_HOT / PROCESSING / BLOCKED）+ VM Pool 利用率
- [ ] 右侧对话区：显示当前选中 Agent 的 `Visibility::Developer`（含 thinking blocks + tool trace）
- [ ] `Ctrl+A` / `Ctrl+N` 快捷键切换 Agent，切换不中断任何 Agent 运行
- [ ] attach / detach 机制：断开 TUI 时（`Ctrl+D`）Agent 继续运行，状态保留
- [ ] 多实例只读 attach：允许多个 TUI 进程同时 attach 同一 Agent（只读旁观，不能发消息）
- [ ] 状态栏：当前 Agent 名、会话轮数、context 使用率、当前 VM 状态

#### C3：Channel Adapter 框架

- [ ] `ChannelAdapter` trait 定义：

```rust
trait ChannelAdapter: Send + Sync {
    fn channel_hints(&self) -> &str;           // 注入 system prompt 的平台描述
    fn visibility_filter(&self) -> Visibility; // 事件过滤规则
    async fn send(&self, event: AgentEvent) -> Result<()>;
}
```

- [ ] Caster 进程 Channel 注册表：运行时动态注册/注销 Channel
- [ ] SessionStart 时：调用 `adapter.channel_hints()`，通过 `SystemPromptBuilder.section("channel_hints", ...)` 追加到 system prompt 末尾（非可缓存区块）
- [ ] 事件广播：DaemonLoop 产生 AgentEvent → 按 visibility 过滤 → 投递到已注册的对应 Channel Adapter
- [ ] `agent.yaml` 中 `channel` 字段（可选，指定默认绑定的 Channel 类型）：

```yaml
channel:
  type: feishu
  webhook_secret: "${FEISHU_WEBHOOK_SECRET}"
```

#### C4：FeishuChannel

**目标**：Feishu 群机器人接入，用户在飞书群 @Agent，Agent 回复飞书卡片消息。

- [ ] Feishu webhook 监听（axum HTTP handler）：接收 `im.message.receive_v1` 事件
- [ ] 消息提取：解析 `content` JSON，支持纯文本 + AT 消息
- [ ] 通过 Unix socket 将消息路由到目标 DaemonLoop
- [ ] `channel_hints()` 返回："你正在通过飞书消息与用户交互。请简洁，避免大段技术输出。飞书支持基础 Markdown，可用 **粗体** 和 `代码`。"
- [ ] `send()` 实现：将 `Visibility::User` 事件的文本内容转为飞书消息卡片，通过飞书 Open API 发送（`POST /open-apis/im/v1/messages`）
- [ ] 消息去重：防止 webhook 重试导致重复发送（记录 `message_id` 集合）
- [ ] 支持多轮对话：同一 `chat_id` 复用同一 DaemonLoop Session

**CHANNEL_HINTS 完整流**：

```
飞书群消息 → FeishuChannel webhook
  → 查找/创建 feishu DaemonLoop Session
  → SessionStart hook:
      SystemPromptBuilder += section("channel_hints", feishu_hints)
  → 消息入队，DaemonLoop PROCESSING
  → LLM 根据 channel_hints 格式化输出（已知在飞书上）
  → AgentEvent(User, TextDelta) → FeishuChannel.send()
  → 飞书卡片消息发送到群
  → Agent 回到 IDLE_HOT，等待下一条消息
```

#### C5：SlackChannel（后续）

- [ ] Slack Events API webhook（`message.im` + `app_mention` 事件类型）
- [ ] `channel_hints()` 返回：mrkdwn 格式说明（`*粗体*`、`` `代码` ``、`>引用`，避免 HTML）
- [ ] `send()` 通过 Slack Web API 发送（`chat.postMessage`，Block Kit 格式）
- [ ] slash command 支持：`/sage <message>` 触发 Agent

#### C6：WebUIChannel（远期）

- [ ] HTTP SSE 长连接：前端订阅 `/api/agents/<name>/events` SSE 流
- [ ] `Visibility::User` 事件转 JSON 推送
- [ ] `channel_hints()` 返回：HTML/Markdown 两用格式提示
- [ ] 配合 Runeforge Web UI 实现浏览器端 Agent 交互

**验收标准**：

```bash
# TUI 多 Agent 面板
sage connect
# 显示所有 Agent 状态列表，切换到 feishu，查看 thinking trace

# Feishu Channel
# 在飞书群 @sage 发消息 "帮我看下明天日程"
# → sage feishu DaemonLoop 收到消息
# → LLM 用简洁飞书格式回复（无 Markdown 长段落）
# → 飞书群收到卡片消息
```

#### C7：触发系统（应用级）

**触发系统是 Sage 应用级基础设施，不属于单个 Agent 的配置。** 触发器只做一件事：构造消息，路由到目标 Agent 的 DaemonLoop 消息队列。

```yaml
# ~/.sage/triggers.yaml（应用级，独立于 agent.yaml）
triggers:
  - name: morning-brief
    type: cron
    schedule: "0 9 * * 1-5"       # 工作日早 9 点
    route_to: feishu-agent
    message: "生成今日飞书日程摘要，发送到飞书"

  - name: feishu-inbound
    type: feishu_event
    event: im.message.receive_v1   # 飞书消息抵达（与 C4 共享 webhook 入口）
    route_to: feishu-agent
    # message 由 Channel Adapter 从事件 payload 中提取，无需手动配置
```

触发类型：

| 类型 | 信号 | 适用场景 |
|---|---|---|
| `cron` | 时间表达式（cron syntax） | 定时任务、日报、周报 |
| `feishu_event` | 飞书 Open Platform 事件 | 飞书消息触发（与 FeishuChannel 共享 webhook） |
| `file_watch` | inotify / FSEvents 文件变化 | 监听 inbox/ 目录有新文件就处理 |

实现要求：
- [ ] `~/.sage/triggers.yaml` 解析，注册到 TriggerEngine
- [ ] `cron` 触发：tokio-cron-scheduler，构造 Message 路由到目标 Agent
- [ ] `feishu_event` 触发：复用 C4 的 webhook handler，触发器路由和 Channel 回复路由分离
- [ ] `file_watch` 触发：notify crate，FSEvents（macOS）/ inotify（Linux）
- [ ] 触发器执行记录写入 `~/.sage/trigger-log.jsonl`（便于审计和调试）
