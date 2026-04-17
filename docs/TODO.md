# Sage 完整 TODO 清单

从 agent-caster 到 Sage 完整产品形态的全量交付清单。综合 `docs/future.md`、`docs/roadmap.md`、`docs/design/` 各设计文档，以及代码现状探索结果。

**起点**：agent-caster, 24,277 行 Rust, 876 tests, 6 crates, 4 LLM providers, 7 tools, 三层循环。

---

## ⚠️ 设计与实现差异说明

以下是当前代码与设计文档之间的已知差异，后续需统一：

| 设计文档命名 | 当前代码命名 | 位置 | 建议 |
|------------|------------|------|------|
| `on_pre_tool_use` | `on_before_tool_call` | engine.rs | 统一为设计文档命名（v0.9.0 重构时一并改） |
| `on_post_tool_use` | `on_after_tool_call` | engine.rs | 同上 |
| `HookEvent` 系统 | 不存在 | — | v0.9.0 H1 新增 |
| `SessionType` | 不存在 | — | v0.7.0 M3 新增 |
| `AgentConfig.hooks` | 不存在 | config.rs | v0.9.0 H1 新增 |
| `AgentConfig.memory` | 不存在 | config.rs | v0.7.0 M3 新增 |
| `AgentConfig.wiki` | 不存在 | config.rs | v0.8.0 新增 |
| `SandboxConfig.workspace_host` | 不存在 | config.rs | v0.7.0 M2 新增 |
| `SandboxConfig.mode` (none/microvm) | 不存在 | config.rs | v0.8.0 新增 |
| `SandboxConfig.rootfs` tier | 不存在 | config.rs | v0.8.0 新增 |

---

## v0.1.0 — First Light ✅

> 第一次端到端跑通：`sage run` 用真实 LLM 完成一个多步编码任务。

### P0: 项目改名 agent-caster → sage ✅

- [x] 目录重命名：6 crates
- [x] Cargo.toml workspace members + 跨 crate 依赖更新
- [x] 代码 `use agent_runtime::` → `use sage_runtime::` 全局替换
- [x] binary 名: `agent-caster` → `sage`
- [x] CLI 重构: `sage run` / `sage serve` 子命令
- [x] CLAUDE.md 同步更新
- [x] `cargo test --workspace` + `cargo clippy` 全过 (853 tests)
- [x] Git: `refactor: rename agent-caster → sage` (84185c8)

### P1: SageEngine 嵌入式 API ✅

- [x] `sage-runtime/src/engine.rs` — SageEngine + SageEngineBuilder
  - Builder pattern: `.system_prompt()` `.provider()` `.model()` `.llm_provider()`
  - 持有 atomic config fields，每次 `run()` 新建 Agent
- [x] `engine.run(msg) -> EventReceiver<AgentEvent>`
- [x] SageTool = `pub use AgentTool as SageTool`，`.register_tool()` 动态注册
- [x] Hooks: `.on_before_tool_call()` / `.on_after_tool_call()` *(设计文档命名：on_pre/post_tool_use)*
- [x] 公共 API: SageEngine, SageEngineBuilder, SageError, SageTool, AgentEvent, EventReceiver
- [x] 21 tests (9 builder + 9 run + 3 resolve)
- [ ] Lifecycle Hooks 扩展 (on_agent_start/on_agent_end/on_error) — 推迟到 v0.9.0 H1

### P2: 端到端联调（无沙箱）✅

- [x] `sage run` CLI 可用，终端流式输出 AgentEvent
- [x] 真实 LLM 调用验证（Qwen）
- [x] 多轮 tool call（>3 轮）
- [x] 并行 tool call
- [x] Steering 队列验证

### Review 技术债

- [x] double AgentEnd / resolve_or_construct_model / RoutingProvider 等 bug 全部修复
- [ ] CancellationToken — 仍未实现
- [ ] 5 个 Arc newtype wrapper boilerplate
- [ ] 全局 registry → Engine 实例级 — 仍未实现

---

## v0.2.0 — Sandboxed ✅

> 工具执行从本地进程切换到 microVM。

### P3: Sandbox 对接 msb_krun ✅

- [x] msb_krun 编译验证（macOS HVF / Linux KVM）
- [x] 最小 rootfs 构建（sage-guest 交叉编译 aarch64-unknown-linux-musl）
- [x] VM 启动链路：SandboxBuilder::create() → VM → Guest Agent PID 1
- [x] Host ↔ Guest virtio-console CBOR 通信
- [x] 7 个工具通过 sandbox 执行（ToolBackend trait：LocalBackend / SandboxBackend）
- [x] Volume mount（只读 / 读写）+ Path traversal 防护
- [x] VM 生命周期管理：创建/停止/Drop 兜底

---

## v0.3.0 — Full LLM Coverage ✅

> 20+ Provider 全量覆盖，对齐 pi-mono。

### P4: LLM Provider 补全 ✅

- [x] Bedrock / Vertex AI / Azure OpenAI
- [x] Groq / Mistral / xAI / DeepSeek / Ollama / GitHub Copilot / Cerebras 等
- [x] openai_compat.rs 通用兼容 provider
- [x] 模型自动发现（`/v1/models` 探测）
- [x] redacted_thinking 往返（ThinkingBlock roundtrip）
- [x] 56 个 mockito-based 集成测试

---

## v0.4.0 — Production Runtime ✅（P5.5 已补全）

> 长对话、配置简化、安全加固。

### P5: 上下文压缩 (Compaction) ✅

- [x] `sage-runtime/src/compaction.rs`（~2962 行，103 tests）
  - token 估算器 + should_compact + find_cut_point + 三套摘要 prompt
  - ContextOverflowHook trait（Compact / Truncate / CustomSummary / Abort）
- [x] 压缩策略：截断 + LLM 摘要
- [x] on_context_overflow hook 集成到 agent_loop.rs

### P6: Runner 增强 — Toolset 预设 ✅

- [x] Toolset 枚举：Coding / Ops / Web / Minimal / Readonly
- [x] AgentConfig YAML `toolset` 字段支持
- [x] Toolset ↔ ToolPolicy 联动

### P5.5: 上下文工程增强 ✅（已全部实现，TODO.md 之前滞后）

**P0 — Microcompact 双层压缩 ✅**

- [x] `compaction.rs` 新增 `microcompact()` 轻量清理层（`microcompact_threshold: f32`，默认 0.75）
- [x] agent_loop.rs 集成：先 microcompact → 还不够 → 再 full compaction
- [x] 零 LLM 调用开销

**P1 — Anthropic Provider Prompt Caching ✅**

- [x] `providers/anthropic.rs`：system prompt block 加 `cache_control: { type: "ephemeral" }`
- [x] 最后一条 user message 加 `cache_control: { type: "ephemeral" }`
- [x] 验证：连续对话中 Usage.cache_read > 0

**P1 — SystemPromptBuilder ✅**

- [x] `sage-runtime/src/system_prompt.rs` — `SystemPromptBuilder` + `PromptSection { name, content, cacheable }`
- [x] `.section()` / `.cacheable_section()` / `.build()`
- [x] Provider 层根据 `cacheable` 标记决定是否加 `cache_control`

**P2 — Context Budget 机制 ✅**

- [x] `ContextBudget` struct（context_window / system_reserve / output_reserve / history_budget）
- [x] `microcompact_threshold: f32` / `compaction_threshold: f32`

**P2 — transformContext Hook ✅**

- [x] SageEngineBuilder `.on_transform_context(Fn(&mut Vec<AgentMessage>))`
- [x] agent_loop.rs LLM 调用前执行

### P7: 安全加固（Layer 3）✅

- [x] seccomp-bpf profile（BPF filter + allowlist）
- [x] Landlock LSM（allowed_paths allowlist）
- [x] 网络策略（airgapped / whitelist / full）
- [x] 资源限制（RLIMIT_NOFILE / RLIMIT_FSIZE / RLIMIT_NPROC + tmpfs 限额）
- [x] GuestSecurityConfig 共享类型 + fail-closed 语义

---

## v0.5.0 — Advanced Sandbox *(推迟到 v1.x)*

> 沙箱高级能力 — OCI 镜像、VM 预热池、快照恢复。
>
> **推迟原因**：v0.8.0 引入 `sandbox.mode: none` 作为本地部署默认模式，个人 Agent 场景不需要这些能力。OCI / VM Pool / Snapshot 是多租户生产场景才用得上的，v1.x 做。

### P8: 高级沙箱（v1.x）

- [ ] OCI 镜像支持（Docker image → OCI → rootfs，预装工具链）
- [ ] VM 预热池（预创建 N 个 idle VM，任务到来时从池中取）
- [ ] VM 快照 / 快速恢复（checkpoint + 秒级恢复）
- [ ] 并发控制（Engine Semaphore + 排队/拒绝策略）

---

## v0.6.0 — Observability & DevEx

> 可观测性 + 构建流水线。（`sage test` 合并到 v0.7.0+ Harness 系统）

### P9: 运维

- [ ] OpenTelemetry 链路追踪（Agent 执行 / LLM 调用 / Tool 执行全链路 span）
- [x] 结构化日志（tracing crate + `SAGE_LOG_FORMAT=json` + `SAGE_LOG`/`RUST_LOG` directive）
- [ ] 健康上报（pressure 指标 + 可选 Rune Runtime 对接）

### P10: 构建流水线 ✅

- [x] macOS HVF entitlements 签名（codesign + `entitlements/sage.entitlements`，CI build.yml 集成）
- [x] 交叉编译：sage-guest (aarch64-linux-musl) + sage-cli (macOS arm64 / Linux x86_64 musl)
- [x] CI 流水线（`.github/workflows/ci.yml`：fmt + clippy + build + test workspace）
- [x] Release 流水线（`.github/workflows/build.yml`：tag 触发，产出三个二进制 artifact）

---

## v0.7.0 — Agent Registry + Workspace + Memory

**目标**：建立 Sage Agent 身份模型。Agent 有名字、持久化工作空间、跨会话记忆。

### M1：Agent 注册与发现

- [x] `~/.sage/agents/<name>/config.yaml` 规范解析（新增 `memory` 字段、`sandbox.workspace_host`）
  - 实际落地：配置文件叫 `config.yaml`（非 `agent.yaml`），且 AGENT.md 与 config.yaml 分离
- [x] `sage init --agent <name>` — 创建 Agent 目录结构 + 模板 config.yaml + AGENT.md + memory/MEMORY.md + workspace/
- [x] `sage list` — 列出所有已注册 Agent（仅 name，未含 description / workspace 大小）
- [x] `sage validate --agent <name>` — 校验 config.yaml 合法性（serde_yaml 解析）
- [ ] `sage list` 展开 description + workspace 体积
- [ ] `sage validate` 额外校验 workspace 路径可访问性

**Workspace 目录结构**：

```
~/.sage/agents/<name>/
├── agent.yaml
└── workspace/
    ├── SCHEMA.md          # Wiki 锚点
    ├── AGENT.md           # 认知框架（注入 system prompt）
    ├── memory/
    │   ├── MEMORY.md      # 索引（注入 system prompt）
    │   └── *.md
    ├── raw/sessions/      # 对话归档（ingest 后标记 processed）
    ├── wiki/
    │   ├── index.md / log.md / overview.md
    │   └── pages/
    ├── assets/
    ├── craft/             # Agent 自管理产物
    └── metrics/           # TaskRecord
```

### M2：Workspace 挂载 ✅

- [x] `AgentConfig.sandbox.workspace_host: Option<PathBuf>`
- [x] tilde 展开（`deserialize_workspace_host` 自定义反序列化器）
- [x] `SandboxBuilder` 将 workspace_host 作为读写 `VolumeMount`，guest path = `/workspace`（`serve.rs` L381/L613）
- [x] landlock `allowed_paths` 自动加入 `/workspace`
- [x] 首次启动时 `fs::create_dir_all` 自动创建 workspace 目录（`init_agent` L101）
- [x] VM 销毁后 workspace 内容保留（Volume mount 语义天然满足）

### M3：知识系统初始化

- [x] `AgentConfig.memory.auto_load: Vec<String>` — 默认 `["AGENT.md", "memory/MEMORY.md"]`
- [x] `AgentConfig.memory.inject_as: MemoryInjectMode` — `PrependSystem | InitialMessage`
  - ⚠️ `InitialMessage` 当前回退为 `PrependSystem`（`serve.rs` L263 有 TODO warning）
- [x] 启动时读取 auto_load 文件，拼成 memory block（`context::prepend_memory_sections`）
- [ ] 通过 `SystemPromptBuilder.cacheable_section("memory", ...)` 注入享受 prompt caching
  - 当前实现是字符串拼接（`context.rs`），未走 `SystemPromptBuilder` 的 cacheable section
- [x] 文件不存在时静默跳过（`load_memory_sections` L201）
- [ ] 首次启动初始化目录结构 + 生成 `SCHEMA.md` + `wiki/index.md` / `log.md` / `overview.md` 空模板
  - 当前 `init_agent` 只建 `AGENT.md` / `memory/MEMORY.md` / `config.yaml` / `workspace/`，不建 wiki 骨架
- [x] Skill 扫描（`workspace/skills/` + `~/.sage/skills/`，按 `*.md` 全量拼接到 system prompt）
- [ ] `.agent/skills/` 平台级路径 + 自动安装 `chasey-myagi/llm-wiki` 4 个 Skills
- [ ] `workspace/craft/` 目录与 SOP frontmatter 解析
- [x] **新增 `sage-runner` 结构体**（`config.rs`：`MemoryConfig` / `MemoryInjectMode` / `SandboxConfig` / `SessionType`）：

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
    // v0.8.0 追加：mode, rootfs
}

pub enum SessionType {
    UserDriven,
    WikiMaintenance,
    CraftEvaluation,
    HarnessRun,        // v0.9.0 Harness 追加
}
```

- [ ] **TaskRecord 采集（MetricsCollector）**：
  - [x] `TaskRecord` 结构体 + ULID task_id（`sage-runner/src/metrics.rs`）
  - [ ] 订阅 AgentEvent 流（MetricsCollector 尚未实现）
  - [ ] 累积每轮 `TurnEnd.message.usage`、工具调用/错误次数、压缩次数（字段定义了但没人写）
  - [ ] UserDriven 会话结束时写 `workspace/metrics/<task_id>.json`
  - [ ] 定期更新 `workspace/metrics/summary.json`（最近 50 条滚动聚合）
  - [ ] WikiMaintenance / CraftEvaluation / HarnessRun 不写 TaskRecord（采集逻辑缺位）
  - ⚠️ 当前 `TaskRecord` 缺字段：`config_hash` / `started_at` / `ended_at` / `duration_ms` / `cache_read_tokens` / `cache_write_tokens` / `tool_error_count` / `failure_reason` / `crafts_active`

```rust
pub struct TaskRecord {
    pub task_id: String,           // ULID
    pub agent_name: String,
    pub model: String,
    pub config_hash: String,       // sha256(agent.yaml)
    pub started_at: u64,
    pub ended_at: u64,
    pub duration_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub turn_count: u32,
    pub tool_call_count: u32,
    pub tool_error_count: u32,
    pub compaction_count: u32,
    pub success: bool,
    pub failure_reason: Option<String>,
    pub crafts_active: Vec<String>,  // Phase 2+
}
```

**验收标准**：

```bash
sage init --agent feishu
sage list                    # feishu  [飞书专员]  workspace: 0B
sage validate --agent feishu # ✓ OK
# workspace/MEMORY.md 写入内容，运行任务后确认注入到 system_prompt
```

---

## v0.8.0 — TUI + Standard Rootfs + Session Persistence

**目标**：`sage chat --agent feishu` 完整跑通多轮交互，daemon 模式，wiki 自动维护。

### M4：TUI 交互（sage chat）

- [x] `sage chat --agent <name>` CLI 命令（`--dev` 跳过 microVM）
- [x] 交互式输入行（tokio stdin + 简易 read-eval loop，非 rustyline）
- [x] 流式输出 LLM 文本（`TerminalSink` 逐 token flush）
- [x] 工具调用可视化：`[tool: name]` / `✗ ERROR` 提示
- [x] 多轮对话：`SageSession` context 累积
- [x] 内置命令：`/exit`、`/reset`
- [ ] `--resume` flag（当前未实现）
- [ ] Ctrl+C 优雅退出 / 错误提示细化（LLM API / 沙箱崩溃 / 工具超时）

### 沙箱模式：microvm + none ✅

- [x] `SandboxConfig.mode: SandboxMode` 枚举（`Microvm | Host`，Host ≈ 设计的 None）
- [x] `mode: host` 下 bash tool 直接在宿主机执行
- [x] `allowed_binaries` 仍然生效（`WEB_BINARIES` 白名单等）
- [x] `sage chat --dev` / `sage run --dev` flag：强制 `mode: host`（`SandboxMode::with_dev_override`）

### Standard Rootfs Bundle（microvm 模式）

- [x] `SandboxConfig.rootfs: RootfsTier` 字段解析（`Minimal | Standard | Custom`）
- [ ] standard tier：curl / python3 / jq / git / rg / fd / gh 静态编译进 rootfs（scripts/build-sandbox.sh 只编 sage-guest，未打 standard rootfs）
- [ ] custom tier：解析 `rootfs_path`，挂载用户 rootfs tar.gz
- [ ] tools/ 持久化卷：`~/.sage/agents/<name>/tools/` → VM 内 `/agent-tools/`
- [ ] `$PATH` 预置 `/agent-tools/bin`

### Daemon 模式（已基本完成 ✅）

- [x] `sage start --agent <name>` — detached 子进程 + `__daemon-server__` 隐藏子命令 + setsid
- [x] daemon 监听 Unix socket：`/tmp/sage-<name>.sock` + PID 文件 `~/.sage/agents/<name>/daemon.pid`
- [x] `sage connect --agent <name>` — 接入 socket 流式 I/O
- [x] `sage disconnect` — 提示用 Ctrl+C / `/exit` 断开
- [x] `sage stop --agent <name>` — 优雅 Shutdown 协议 + SIGTERM 回退
- [x] `sage send --agent <name> --message "..."` — 非交互式
- [x] `sage status` — 列出 daemon（name + PID + running/stopped）
- [ ] `sage status` 展开运行时长 + 对话轮数（当前仅 PID + 状态）
- [ ] daemon 崩溃后自动写 crash log（当前直接进程退出）

### 知识蒸馏（PreCompact hook 触发）

- [ ] PreCompact event 触发时，hook 提示 Agent：先更新 memory/MEMORY.md，再压缩
  - 当前只有 PreToolUse / PostToolUse / Stop 三种 hook，无 PreCompact
- [ ] system prompt 中告知 Agent：随时主动写入 `/workspace/memory/`
- [ ] compact 后 Memory 仍然在 system prompt（每次 compact 重新注入）

### Wiki 自动维护（IDLE hook 触发）

- [ ] UserDriven 会话结束后，归档到 `raw/sessions/`（无 daemon 端 wiki 采集逻辑）
- [ ] PostProcessing hook 检查未处理 session 数量 ≥ `wiki.trigger_sessions`
- [ ] 触发 WikiMaintenance 会话：Agent 使用 wiki_* 工具提炼知识
- [ ] WikiMaintenance 期间用户消息进入优先级队列
- [ ] wiki 维护冷却期：两次维护至少间隔 `wiki.cooldown_secs`
  - ⚠️ 仅有 `SessionType::WikiMaintenance` 枚举值（config.rs L470），DaemonLoop / IDLE 触发机制尚未实现

**验收标准**：

```bash
sage start --agent feishu
sage connect --agent feishu  # 多轮对话
sage status                  # feishu  PID:12345  运行 2h15m  12 轮对话
sage stop --agent feishu
```

---

## v0.9.0 — Hook & Skill 系统

**目标**：Agent 具备可扩展生命周期钩子和能力模块，对齐 CC Hook/Skill 机制精简版。

### H1：Hook 系统（已实现 3/12 事件）

- [ ] `HookEvent` 枚举（12 个事件）：

| Event | 优先级 | 状态 | 用途 |
|-------|-------|------|------|
| `SessionStart` | P0 | [ ] | 初始化环境、注入动态上下文 |
| `UserPromptSubmit` | P0 | [ ] | 拦截/增强用户输入 |
| `PreToolUse` | P0 | [x] | 工具调用前审计/安全防护（exit 2 = 阻断） |
| `PostToolUse` | P0 | [x] | 工具调用后回调/副作用触发 |
| `Stop` | P0 | [x] | Agent 结束前质量检查（exit 2 = 要求继续）|
| `SessionEnd` | P1 | [ ] | 保存状态、清理外部资源 |
| `PreCompact` | P1 | [ ] | 压缩前处理（exit 2 = 阻断压缩） |
| `PostCompact` | P1 | [ ] | 压缩后通知 |
| `FileChanged` | P1 | [ ] | 监听 workspace 文件变化 |
| `MemoryUpdated` | P2 | [ ] | MEMORY.md 写入后触发 |
| `AgentSwitched` | P2 | [ ] | `/switch <agent>` 切换时 |
| `WorkspaceChanged` | P2 | [ ] | 工作目录变更 |

- [ ] 统一 `HookEvent` 枚举（当前是三个独立 trait：`BeforeToolCallHook` / `AfterToolCallHook` / `StopHook`）
- [ ] tokio broadcast channel 广播系统（当前直接 trait dispatch，无 broadcast）
- [x] **command hook 执行引擎**：`hooks::execute_hook`（`/bin/sh -c` fork + kill_on_drop + 超时）
- [x] **exit 2 协议**：PreToolUse exit 2 阻断 + stderr steering / Stop exit 2 Continue feedback
- [ ] exit 0 stdout 按事件语义注入（当前 stdout 丢弃，只看 stderr）
- [ ] Stdin JSON contract（Stop hook）：当前通过环境变量 `SAGE_*` 传参，非 stdin JSON
- [x] `config.yaml` 中 `hooks` 字段解析（`HooksConfig` 支持 `pre_tool_use` / `post_tool_use` / `stop`）
- [ ] 扩展解析：`SessionStart` / `UserPromptSubmit` / `PreCompact` / `PostCompact` / `SessionEnd` 等 9 个事件
- [ ] 全局 `~/.sage/hooks.yaml` 加载 + 与 agent 级 hooks 合并
- [ ] Matcher 过滤（tool name 前缀匹配）+ `if` 条件（权限规则语法子集）
  - 当前 `HookConfig` 只有 `command` + `timeout_secs`，无 matcher / if
- [ ] http hook / prompt / agent hook（当前只支持 command）
- [ ] **API 命名统一**：`on_before_tool_call` / `on_after_tool_call` → `on_pre_tool_use` / `on_post_tool_use`（仍是旧名）
- [ ] on_agent_start / on_agent_end / on_error lifecycle hooks（v0.1.0 遗留）

### H2：Harness 系统（`sage test`）

Harness 通过 Stop hook 作为评估入口，exit 2 强制 Agent 继续迭代，实现质量门禁闭环。

- [x] **HarnessConfig YAML 支持**（config.yaml 内联，`HarnessConfig { evaluator, timeout_secs }`）
- [x] 独立测试套件 YAML 解析（`TestSuite { suite, agent, cases[{name,message,eval,max_turns}] }`）
- [ ] criteria 检查协议（`output_contains` / `tool_called` / `token_budget`）— 当前只支持 `eval` 脚本出口码，无声明式 criteria
- [x] `sage test --suite <path>` CLI 命令
- [ ] `--case "..."` 单例过滤
- [ ] `--parallel N` 并发
- [ ] `--reporter junit` / `--output results/`（当前仅 `terminal` / `json`）
- [x] Eval script 协议：exit 0 Pass / exit 2 Fail + stderr / 其他 Error（`harness::run_eval_script`）
- [x] Stop hook 通过环境变量 `SAGE_EVENT=Stop` + `SAGE_AGENT_NAME` / `SAGE_SESSION_ID` / `SAGE_TURN_COUNT` / `SAGE_STOP_REASON` / `SAGE_MODEL` / `SAGE_LAST_ASSISTANT_MESSAGE` 传参
- [ ] Stop hook stdin JSON contract（当前是 env vars，不是 stdin JSON，与设计文档不一致）
- [x] `SessionType::HarnessRun` 枚举值已定义（config.rs L470）
- [ ] 运行时让 HarnessRun 不写 TaskRecord（尚无 TaskRecord 写入逻辑，整体缺失）
- [ ] 测试结果包含 token 用量（当前只有 turns + duration_ms）
- [ ] Craft 效率验证（Craft 系统整体未实现）
- [ ] DaemonLoop 中 HarnessRun session 类型不触发 wiki 维护

### S1：Skill + Craft 系统（两级架构）

**全局 Skills**（`~/.sage/skills/`）= Markdown + YAML frontmatter，`/skill-name` 调用时注入当前对话，只读。
**Craft**（`workspace/craft/`）= Agent 自管理可复用产物（SOP/脚本/模板/素材），通过 `craft_manage` 工具管理。

- [x] 启动时扫描 `~/.sage/skills/` + `workspace/skills/`（注意：实际路径是 `skills/` 不是 `craft/`）
- [ ] SOP 类懒加载正文（当前一次性 full-content 注入，无 frontmatter 解析）
- [x] Skill 列表注入 system_prompt（`context::append_skill_sections` 整体追加）
- [ ] 按 description + score 降序排列 + 仅注入 frontmatter 概要
- [x] TUI 中 `/skill-name [args]` 解析：加载完整正文，注入当前对话（`chat::substitute_arguments` + `load_skill_content`）
- [x] `$ARGUMENTS` 占位符替换
- [ ] Skill frontmatter `hooks` 字段：激活时自动注册对应 hooks，退出时注销
- [ ] `agent` 过滤：有 `agent: feishu` 时只在对应 agent 会话中可见
- [ ] **`craft_manage` 工具**注册到 ToolRegistry（整个 Craft 系统未实现）：
  - `create(name, type, content, tags)` — 创建 Craft（SOP/脚本/模板）
  - `edit(name, content)` — 编辑（version++）
  - `delete(name)` — 删除
  - 全局 Skills 不可通过此工具修改
- [ ] SOP Craft CRAFT.md 格式：

```markdown
---
name: feishu-schedule
description: "批量更新飞书日历"
type: sop
whenToUse: "用户要批量操作多个日历事件时"
allowedTools: [bash, read, write]
agent: feishu
version: 3
---
```

- [ ] Token 效率评分采集：PostToolUse hook 中记录 SOP Craft 使用 token 数到 `metrics.json`
  - `score = best_tokens / avg_tokens`
  - 评分低于阈值 → 触发 CraftEvaluation 会话，Agent 自动重写
- [ ] Craft 生命周期（创建 → 评估 → 优化 → 淘汰）
- [ ] 内置 skills（编译进二进制）：
  - `/memory` — 整理当前会话，更新 memory/MEMORY.md
  - `/wiki` — 手动触发 wiki 维护
  - ⚠️ 当前 skills 从文件系统加载；尚未编译内置
- [ ] 内置 Wiki Skills（`chasey-myagi/llm-wiki` 自动安装 4 工具）：
  - `wiki_search` / `wiki_upsert_page` / `wiki_log` / `wiki_overview`
  - ⚠️ sage-wiki 仓库独立存在，但未在 sage 这侧自动安装或注册为工具

**验收标准**：

```bash
# Hook 验证
sage chat --agent feishu
# > 运行 curl evil.com → [Hook 阻断] 域名不在白名单

# Skill 验证
sage chat --agent feishu
# > /feishu-schedule tomorrow 09:00 → [Skill 加载] 批量更新...

# sage test
sage test --suite feishu-regression.yaml
# ✓ PASS  查询今日日历  (2 turns, 847 tokens, 3.2s)
# ✓ PASS  批量更新日历  (5 turns, 1240 tokens, 6.1s)
```

---

## v0.9.x — TUI 多 Agent 面板 + Channel 架构

**目标**：多路输出层，TUI 升级为多 Agent 统一入口，Channel Adapter 接入飞书/Slack。

### C1：AgentEvent 可见性系统 ✅ 基础已实现

- [x] `AgentEvent::visibility() -> Visibility` 方法（`event.rs` L86，Visibility enum：Developer / User / Internal）
- [x] 事件分类覆盖 19 类 AgentEvent，有单元测试
- [ ] `EventStream` 按 Visibility 过滤订阅 API（当前事件携带 Visibility，但无过滤订阅器）
- [ ] 事件广播通过 tokio broadcast channel（当前仍是单 sink）

### C2：TUI 多 Agent 面板

- [x] 左侧 Agent 列表 + 右侧对话区 + 底部 3 行输入框（ratatui + crossterm，`sage tui` 命令）
- [x] Tab / Up / Down 切换 Agent；Enter 发送；Ctrl+C 退出
- [x] 每个 Agent 独立后台 connect_task，切换不中断运行
- [ ] 展示 daemon 状态（IDLE_HOT / PROCESSING / BLOCKED）+ VM Pool 利用率
- [ ] 按 `Visibility::Developer` 过滤对话区事件流
- [ ] 多实例只读 attach（多个 TUI 同时 attach 同一 Agent）
- [ ] 状态栏：会话轮数 / context 使用率 / VM 状态

### C3：Channel Adapter 框架

- [x] `ChannelAdapter` trait 骨架（`sage-runner/src/channel/mod.rs`，当前 `send(&str)` + `name()`）
- [ ] 扩展到设计 API：`channel_hints()` / `visibility_filter()` / `send(event: AgentEvent)`
- [ ] Caster 进程 Channel 注册表（运行时动态注册/注销）
- [ ] SessionStart 调用 `channel_hints()` 通过 `SystemPromptBuilder.section("channel_hints", ...)` 追加
- [ ] 事件广播：DaemonLoop AgentEvent → 按 visibility 过滤 → 投递到 Channel Adapter
- [ ] `config.yaml` 中 `channel` 字段解析

### C4：FeishuChannel（仅 stub，未接入）

- [ ] Feishu webhook 监听（axum HTTP handler）：接收 `im.message.receive_v1` 事件
- [ ] 消息提取：解析 `content` JSON，支持纯文本 + AT 消息
- [ ] 通过 Unix socket 路由到目标 DaemonLoop
- [ ] `channel_hints()` 返回：飞书平台格式提示
- [ ] `send()` 真实接入（当前只是 `tracing::info!` 打印 stub 日志）
- [ ] 消息去重（记录 `message_id` 集合防止 webhook 重试重复发送）
- [ ] 同一 `chat_id` 复用同一 DaemonLoop Session

### C5：SlackChannel（后续）

- [ ] Slack Events API webhook（`message.im` + `app_mention`）
- [ ] `channel_hints()` 返回 mrkdwn 格式说明
- [ ] `send()` 通过 Slack Web API（`chat.postMessage`，Block Kit 格式）
- [ ] slash command：`/sage <message>` 触发 Agent

### C6：WebUIChannel（远期）

- [ ] HTTP SSE 长连接：`/api/agents/<name>/events` SSE 流
- [ ] `Visibility::User` 事件转 JSON 推送
- [ ] 配合 Runeforge Web UI 实现浏览器端 Agent 交互

### C7：触发系统（应用级，cron + every_secs 已完成 ✅）

- [x] `~/.sage/triggers.yaml` 解析（`TriggersFile { triggers: Vec<TriggerConfig> }`）
- [x] `cron` 触发（自研 5 字段解析器 + `chrono::Local`）
- [x] `every_secs` 周期触发
- [x] 消息通过 Unix socket 路由到目标 daemon（复用 daemon 协议）
- [x] `sage triggers start` / `sage triggers list` CLI 子命令
- [ ] `feishu_event` 触发（依赖 C4 真实实现）
- [ ] `file_watch` 触发（notify crate / FSEvents / inotify）
- [ ] 触发器执行记录写入 `~/.sage/trigger-log.jsonl`
- ⚠️ 设计文档字段是 `schedule` / `route_to`，实现是 `cron` / `agent`，需统一

**验收标准**：

```bash
# TUI 多 Agent 面板
sage connect   # 显示所有 Agent 状态列表

# Feishu Channel
# 飞书群 @sage → DaemonLoop 收到 → LLM 回复 → 飞书卡片消息

# 触发系统
# 工作日早 9 点自动触发 morning-brief → feishu-agent 生成日程摘要
```

---

## v1.0.0 — Release

> 可嵌入执行引擎 + 独立 Agent 产品完整发布。

### P12: 文档 & 示例

- [ ] README.md 重写（Sage 定位 + Quick Start）
- [ ] API 文档（rustdoc：SageEngine / SageTool / AgentEvent / Hook）
- [ ] 示例项目：
  - `examples/minimal.rs` — 最简嵌入（5 行跑通）
  - `examples/custom_tool.rs` — 自定义工具
  - `examples/hooks.rs` — Hook 系统
  - `examples/runeforge_integration.rs` — Runeforge 集成

### P13: 发布准备

- [ ] crates.io 发布（MIT/Apache-2.0，版本 1.0.0，各 crate 描述）
- [ ] GitHub Release（Changelog + 预编译二进制）
- [ ] 公告博文（可选）

### v1.0.0 验收

```rust
// 嵌入使用
let engine = SageEngine::from_config("assistant.yaml").await?
    .register_tool(MyTool::new())
    .on_pre_tool_use(|name, input| HookAction::Allow);

let mut stream = engine.run("完成这个任务").await?;
while let Some(event) = stream.next().await { /* ... */ }
```

```bash
# CLI 独立使用
sage run --config coding-assistant.yaml --message "..."

# Rune 分布式调度
sage serve --runtime localhost:50070

# 多轮交互
sage chat --agent feishu

# Daemon 模式
sage start --agent feishu && sage connect --agent feishu

# Harness 测试
sage test --suite regression.yaml --reporter junit
```

---

## 版本路线图总览

```
v0.1.0 — First Light         ✅  改名 + Engine API + 端到端
v0.2.0 — Sandboxed           ✅  microVM + 工具沙箱
v0.3.0 — Full LLM            ✅  20+ Provider
v0.4.0 — Production Runtime  ✅  压缩 + 上下文工程 + 安全加固
v0.6.0 — Observability       ◐   P10 构建流水线 ✅ / P9 仅 JSON 日志（OTel + 健康上报仍缺）
v0.7.0 — Agent Registry      ✅  M1/M2 ✅ / M3 ✅（Sprint 5: MetricsCollector + Skill frontmatter + init_agent wiki 骨架）
v0.8.0 — TUI + Daemon        ◐   sage chat + daemon + SandboxMode ✅ + PreCompact ✅（Sprint 6）+ Wiki 自维护触发 ✅（Sprint 7）/ Standard rootfs 缺
v0.9.0 — Hook & Skill        ◐   Hook 体系 8/12 ✅（Sprint 6）/ Harness criteria + parallel + junit ✅（Sprint 9）/ Craft S10.3 crafts_active 数据面 ✅ / S10.1/S10.2/S10.4 工具面缺（v1.0.1 follow-up）
v0.9.x — Multi-Agent + Chan. ✅  Visibility + TUI + cron triggers ✅ / Channel 真实 Feishu 接入 ✅（Sprint 8: webhook + HMAC 签名 + card send）
v1.0.0 — Release             ○   文档 + 示例 + crates.io 发布（核心功能已就绪，仅剩文档打磨）
v1.x   — Advanced Sandbox    ○   OCI + VM 预热池 + 快照（多租户生产）
```

| 版本 | 核心交付 | 关键新增 |
|------|---------|---------|
| v0.5.0 | 可观测 | OTel / CI pipeline |
| v0.7.0 | Agent 身份 | Registry / Workspace / Memory / TaskRecord |
| v0.8.0 | 完整交互 | sage chat / daemon / wiki / Standard rootfs |
| v0.9.0 | 可扩展性 | Hook / Harness / Craft / sage test |
| v0.9.x | 多路接入 | Channel / 多 Agent TUI / 触发系统 |
| v1.0.0 | 发布 | 文档 / 示例 / crates.io |
| v1.x | 高级沙箱 | OCI / VM pool / snapshot（多租户场景） |
