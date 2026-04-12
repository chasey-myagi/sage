# Sage v1.0.0 Roadmap

从 agent-caster 到 Sage v1.0.0 — `docs/future.md` 完整最终形态的交付清单。

**起点**：agent-caster, 24,277 行 Rust, 876 tests, 6 crates, 4 LLM providers, 7 tools, 三层循环, Before/AfterToolCall hooks.

---

## v0.1.0 — First Light

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

- [x] `sage-runtime/src/engine.rs` — SageEngine + SageEngineBuilder (~1037 行)
  - Builder pattern: `.system_prompt()` `.provider()` `.model()` `.llm_provider()`
  - 持有 atomic config fields，每次 `run()` 新建 Agent
- [x] `engine.run(msg) -> EventReceiver<AgentEvent>`
  - RoutingProvider 按 model.api 分发到正确 ApiProvider
  - ChannelSink 桥接 AgentEventSink → EventStream
- [x] SageTool = `pub use AgentTool as SageTool`
  - `.register_tool(impl AgentTool)` 动态注册
- [x] Hooks: `.on_before_tool_call()` / `.on_after_tool_call()`
- [x] 公共 API: SageEngine, SageEngineBuilder, SageError, SageTool, AgentEvent, EventReceiver
- [x] 21 tests (9 builder + 9 run + 3 resolve)
- [ ] Lifecycle Hooks 扩展 (on_agent_start/on_agent_end/on_error) — deferred to v0.1.1

### P2: 端到端联调（无沙箱）✅

- [x] `sage run` CLI 可用，终端流式输出 AgentEvent
- [x] **真实 LLM 调用验证**（Qwen）— test_real_qwen_single_turn, API key gated
- [x] 多轮 tool call（>3 轮）— run_multi_turn_tool_calls_four_turns (4 轮 + final)
- [x] 并行 tool call — run_parallel_tool_calls_two_tools_one_turn (2 tools, join_all)
- [x] Steering 队列验证 — run_steering_message_appears_in_events + reaches_provider_context

### Review 发现的技术债

- [x] ~~double AgentEnd emit in error path~~ (fixed)
- [x] ~~resolve_or_construct_model called twice~~ (fixed)
- [x] ~~RoutingProvider silent fallback~~ (fixed)
- [x] ~~`register_builtin_providers()` 每次 `run()` 调用~~ (fixed: `std::sync::Once` guard)
- [x] ~~RoutingProvider error 缺少 Done 事件~~ (fixed: Error + Done{StopReason::Error})
- [x] ~~`builtin_tools()` 静默忽略未知 tool name~~ (fixed: tracing::warn)
- [x] ~~RoutingProvider 路径零测试覆盖~~ (fixed: routing_provider_missing_api_returns_error_and_done)
- [x] ~~CLI binary 名 sage-caster → sage~~ (fixed: [[bin]] name="sage" + package rename sage-cli)
- [x] ~~SandboxBuilder guest path agent-guest → sage-guest~~ (fixed)
- [x] ~~serve.rs 不打印 final assistant reply~~ (fixed: 从 AgentEnd messages 提取并 println)
- [x] ~~CLAUDE.md 中 `cargo run -p sage` 命令过期~~ (fixed: 更新为 sage-cli)
- [ ] 无 cancellation 机制（CancellationToken）— v0.1.1
- [ ] 5 个 Arc newtype wrapper boilerplate — 需改 Agent 接口
- [ ] test_helpers 对外部 crate 不可见 — 需 feature flag
- [ ] 全局 registry → 考虑改为 Engine 实例级 — v0.2.0

### v0.1.0 验收

```bash
sage run --config configs/coding-assistant.yaml \
    --message "创建一个 Rust hello world 项目并运行它"
# Agent 完成多步任务，流式输出事件，正常退出
```

---

## v0.2.0 — Sandboxed

> 工具执行从本地进程切换到 microVM。Sage 区别于普通 Agent 框架的核心。

### P3: Sandbox 对接 msb_krun ✅

- [x] msb_krun 编译验证（macOS HVF / Linux KVM）
- [x] 最小 rootfs 构建
  - sage-guest 交叉编译 aarch64-unknown-linux-musl (static ELF)
  - rootfs = sage-guest binary + busybox
  - 构建脚本自动化 (`scripts/build-sandbox.sh`)
- [x] VM 启动链路 (P3-A)
  - SandboxBuilder::create() → VM 启动 → Guest Agent PID 1
  - Host ↔ Guest virtio-console CBOR 通信验证
- [x] 7 个工具通过 sandbox 执行 (P3-B)
  - ToolBackend trait 抽象 (LocalBackend / SandboxBackend)
  - bash → `sandbox.shell()` → stdout
  - read/write/edit → `sandbox.fs_read/fs_write`
  - grep/find/ls → VM 内执行
- [x] Volume mount（只读 / 读写）(P3-C)
  - VolumeMount serde → SANDBOX_VOLUMES → virtiofs → SAGE_VOLUMES → guest mount
  - Path traversal 防护, MS_NOEXEC 默认 flag
- [x] VM 生命周期管理 (P3-D)
  - SandboxSettings + SageEngineBuilder::sandbox()
  - 任务开始 → 创建 VM / 任务结束 → stop + 清理
  - Drop 兜底 SIGKILL

### v0.2.0 验收

```bash
sage run --config configs/coding-assistant.yaml \
    --message "在 /workspace 创建文件并验证内容"
# 工具在 microVM 内执行，Host 无感知
```

---

## v0.3.0 — Full LLM Coverage

> 20+ Provider 全量覆盖，对齐 pi-mono。

### P4: LLM Provider 补全

**Tier 1（云厂商变体）：**

- [ ] Bedrock provider — AWS SigV4 签名 + Anthropic on Bedrock streaming
- [ ] Vertex AI provider — Google OAuth2 + Gemini on Vertex streaming
- [ ] Azure OpenAI provider — Azure AD / API key + deployment endpoint

**Tier 2（独立 Provider 或通过 OpenAI Compat）：**

- [x] Groq（15 models, detect_compat handles compat dynamically）
- [x] Mistral
- [x] xAI (Grok)（24 models, detect_compat handles compat dynamically）
- [x] DeepSeek — pi-mono 无独立 provider，模型通过 OpenRouter/Groq/Bedrock 提供
- [x] Ollama / LM Studio / vLLM — 通过 OpenAI compat + discover_models() 支持，无需 catalog 条目
- [ ] GitHub Copilot
- [x] Together / Fireworks / SambaNova — pi-mono 无独立 provider，通过 OpenRouter 提供
- [x] Cerebras（4 models, detect_compat handles as is_non_standard）

**基础设施：**

- [x] openai_compat.rs 通用 OpenAI 兼容 provider 对齐 pi-mono
  - ThinkingBlock roundtrip via signature-based dynamic field name
  - tool_name support (requiresToolResultName), hasToolHistory fallback
  - strict=false on tool defs, skip empty assistant, finish_reason mapping
  - 12 new tests, code review 8.83/10 (debea21)
- [x] 模型自动发现（`/v1/models` 端点探测）
  - discover_models() with 15s timeout, discover_models_with_client()
  - DiscoveredModel serde + construct_model_from_discovered (8K conservative ctx)
  - DiscoveryError enum, tracing::debug for skipped entries
  - 17 new tests (mockito mock server), code review 8.48/10 (690c77e)
- [x] redacted_thinking 往返（LlmMessage 增加 thinking variant）
  - ThinkingBlock struct + thinking_blocks field on LlmMessage::Assistant
  - ThinkingBlockEnd event + per-block accumulator
  - Anthropic serialize/deserialize for thinking/redacted_thinking
  - Full roundtrip chain: SSE → accumulator → Content → LlmMessage → API
  - 24 new tests, code review 8.90/10 (85b4009)

### v0.3.0 验收

- 每个 Provider 至少一个集成测试（真实 API 或 mock）
- `sage run --config X.yaml` 可切换所有已支持 Provider

---

## v0.4.0 — Production Runtime

> 长对话、配置简化、安全加固 — 生产环境所需的运行时能力。

### P5: 上下文压缩 (Compaction)

- [ ] `sage-runtime/src/compaction.rs`
  - token 计数器（基于 model context_window）
  - 超限检测
- [ ] 压缩策略
  - 截断：保留 system + 最近 N 条
  - LLM 摘要：调 LLM 压缩历史
- [ ] on_context_overflow hook（上层可注入自定义策略）
- [ ] 集成到 agent_loop.rs INNERMOST 循环 transformContext 步骤

### P6: Runner 增强 — Toolset 预设

- [ ] Toolset 定义：coding / ops / web / minimal
- [ ] AgentConfig YAML 支持 `toolset: coding` 简写
- [ ] Toolset ↔ ToolPolicy 联动（预设自带 allowed_binaries / allowed_paths）

### P7: 安全加固（Layer 3）

- [ ] seccomp-bpf profile
  - Guest Agent 启动加载 filter
  - 只允许必要系统调用，拒绝 ptrace/mount/reboot
- [ ] Landlock LSM
  - Guest Agent 启动设置文件系统访问范围
- [ ] 网络策略
  - airgapped（默认）：Guest 完全断网
  - tsi_whitelist：msb_krun TSI 域名白名单
  - full（仅调试）
- [ ] 资源限制
  - vCPU / memory_mib 从 AgentConfig 读取
  - exec timeout 强制 kill
  - tmpfs 磁盘限额
- [ ] 安全测试
  - `rm -rf /` → Host 不受影响
  - `curl evil.com` → airgapped 拦截
  - fork bomb → vCPU 限制

### v0.4.0 验收

- 30+ turns 长对话自动压缩不 OOM
- seccomp/Landlock 在 Guest 生效
- airgapped 模式下 Guest 无法外联

---

## v0.5.0 — Advanced Sandbox

> 沙箱高级能力 — OCI 镜像、VM 预热池、快照恢复。

### P8: 高级沙箱

- [ ] OCI 镜像支持
  - 用户自定义 rootfs（Docker image → OCI → rootfs）
  - 预装 Python / Node / Rust 工具链的镜像
- [ ] VM 预热池
  - 预创建 N 个 idle VM
  - 任务到来时从池中取，减少冷启动延迟
- [ ] VM 快照 / 快速恢复
  - checkpoint VM 状态
  - 从快照恢复（秒级）
- [ ] 并发控制
  - Engine Semaphore（max_concurrent）
  - 超限排队 / 拒绝策略

### v0.5.0 验收

- OCI 镜像构建 + 使用流程跑通
- VM 预热池冷启动延迟 < 500ms
- 并发 3 个 Agent 互不干扰

---

## v0.6.0 — Observability & DevEx

> 可观测性 + 开发体验。

### P9: 运维

- [ ] OpenTelemetry 链路追踪
  - Agent 执行全链路 span
  - LLM 调用 span（provider / model / latency / tokens）
  - Tool 执行 span
- [ ] 结构化日志
  - tracing crate 集成
  - JSON 格式输出
  - 日志级别可配（AgentConfig / env）
- [ ] 健康上报
  - pressure 指标（CPU / 内存 / 并发数）
  - 与 Rune Runtime 上报对接（可选）

### P10: 构建流水线

- [ ] macOS HVF entitlements 签名（codesign）
- [ ] 交叉编译
  - sage-guest: aarch64-unknown-linux-musl
  - sage-cli: macOS arm64 + Linux x86_64
- [ ] CI 流水线
  - cargo test + clippy + fmt
  - 交叉编译产物
  - 集成测试（mock provider）

### P11: sage test 子命令

- [ ] 测试用例格式（YAML: input → expected behavior）
- [ ] 批量运行 + 报告输出
- [ ] CI/CD 集成

### v0.6.0 验收

- OTel 链路可在 Jaeger 中查看
- `sage test` 跑通测试套件
- 交叉编译产物可直接部署

---

## v1.0.0 — Release

> 可嵌入的 AI Agent 执行引擎，完整最终形态。

### P12: 文档 & 示例

- [ ] README.md 重写（Sage 定位 + SQLite 类比 + Quick Start）
- [ ] API 文档（rustdoc: SageEngine / SageTool / AgentEvent / Hook）
- [ ] 示例项目
  - `examples/minimal.rs` — 最简嵌入（5 行跑通）
  - `examples/custom_tool.rs` — 自定义工具注册
  - `examples/hooks.rs` — Lifecycle Hooks
  - `examples/runeforge_integration.rs` — Runeforge 集成
- [ ] Architecture Guide（从 future.md 精简）

### P13: 发布准备

- [ ] crates.io 发布
  - License (MIT / Apache-2.0)
  - 版本号 1.0.0
  - 各 crate 描述 + keywords
- [ ] GitHub Release
  - Changelog
  - 预编译二进制（macOS arm64 / Linux x86_64）
- [ ] 公告博文（可选）

### v1.0.0 验收

future.md 全部功能点可用：

```rust
// 上层项目嵌入 — 5 行核心代码
let engine = SageEngine::from_config("assistant.yaml").await?
    .register_tool(MyTool::new())
    .on_pre_tool_use(|name, input| { /* audit */ HookAction::Allow });

let mut stream = engine.run("完成这个任务").await?;
while let Some(event) = stream.next().await { /* ... */ }
```

```bash
# CLI 独立使用
sage run --config coding-assistant.yaml --message "..."

# Rune 分布式调度
sage serve --rune localhost:50070

# CI/CD 测试
sage test --suite regression.yaml
```

---

## 版本路线图总览

```
当前 (agent-caster)
  │
  ▼
v0.1.0 — First Light        改名 + Engine API + 端到端跑通（无沙箱）
  │
  ▼
v0.2.0 — Sandboxed          microVM 对接，工具在 VM 内执行
  │
  ▼
v0.3.0 — Full LLM           20+ Provider 全量覆盖
  │
  ▼
v0.4.0 — Production Runtime 压缩 + Toolset + 安全加固
  │
  ▼
v0.5.0 — Advanced Sandbox   OCI 镜像 + VM 预热池 + 快照
  │
  ▼
v0.6.0 — Observability      OTel + 日志 + 构建流水线 + sage test
  │
  ▼
v1.0.0 — Release            文档 + 示例 + crates.io 发布
```

| 版本 | 核心交付 | 预估新增代码 |
|------|---------|-------------|
| v0.1.0 | 改名 + Engine API + 端到端 | ~2,000 行 |
| v0.2.0 | Sandbox 对接 | ~1,500 行 |
| v0.3.0 | 20+ Provider | ~6,000 行 |
| v0.4.0 | 压缩 + 安全 | ~2,500 行 |
| v0.5.0 | 高级沙箱 | ~1,500 行 |
| v0.6.0 | 可观测 + DevEx | ~1,500 行 |
| v1.0.0 | 文档 + 发布 | ~500 行 |
| **累计** | | **~15,500 行** |
| **最终总量** | | **~40,000 行** (现有 24,277 + 新增) |
