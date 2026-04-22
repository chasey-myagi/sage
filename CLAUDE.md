# Sage

领域专家 Agent 应用 — 给任意领域配置一个有独立工作空间、有记忆沉淀的专员。基于 msb_krun microVM 沙箱，通过 TUI 交互或 Runeforge Caster 调度。

## 项目结构

```
crates/
├── sage-cli/        # CLI 入口 (clap CLI + Rune SDK stub)
│   └── src/
│       ├── main.rs  # CLI 解析 + 启动
│       └── serve.rs # Rune 注册 + 任务处理
├── sage-runtime/    # Agent 执行引擎 (LLM + Tools + Events)
│   └── src/
│       ├── engine.rs    # SageEngine (builder + run)
│       ├── agent_loop.rs # Agent 循环
│       ├── event.rs     # AgentEvent 类型
│       ├── llm/         # LLM Providers
│       └── tools/       # 内置工具
├── sage-sandbox/    # Host 端沙箱 SDK
│   └── src/
│       ├── builder.rs   # SandboxBuilder (VM 配置)
│       ├── handle.rs    # SandboxHandle (exec/fs 操作)
│       ├── relay.rs     # AgentRelay (virtio-console 通信)
│       └── error.rs
├── sage-protocol/   # Host↔Guest 线协议
│   └── src/
│       ├── messages.rs  # HostMessage / GuestMessage
│       └── wire.rs      # 长度前缀 CBOR 编解码
├── sage-guest/      # Guest Agent (VM 内 PID 1)
│   └── src/
│       ├── main.rs  # init + 主循环
│       ├── init.rs  # mount filesystems
│       ├── exec.rs  # 命令执行
│       └── fs.rs    # 文件操作
└── sage-runner/     # Agent 配置 + 策略
    └── src/
        ├── config.rs    # AgentConfig (YAML 解析)
        └── tools.rs     # ToolPolicy (白名单校验)
```

## Build & Run

```bash
cargo build
cargo test
cargo run -p sage-cli -- run --config configs/coding-assistant.yaml --message "hello"
cargo run -p sage-cli -- serve --runtime localhost:50070
```

## 依赖

- msb_krun — 纯 Rust microVM (libkrun Rust 化)
- Rune TS/Rust SDK (Phase 2)
- Rune Runtime 需先启动

## 代码对齐原则

`sage-runtime` crate 中的 LLM Provider 代码（`src/llm/providers/`、`src/llm/stream.rs`、`src/llm/types.rs` 等）**必须严格对齐 pi-mono 的 TypeScript 参考实现**。

参考源码位置：`~/Dev/cc/external/pi-mono/packages/ai/src/providers/`

### 具体要求

- **功能逻辑 1:1 移植**：pi-mono 中的每个功能分支（thinking format、cache control、compat detection、usage 计算、SSE 事件处理等）在 Rust 侧都必须有对应实现
- **修改前先对比参考**：修改 provider 代码前，先读对应的 pi-mono `.ts` 文件，确认逻辑一致
- **新增功能同步**：pi-mono 侧新增的 provider 功能应及时同步到 Rust 实现
- **不要自行发明**：不要自创与 pi-mono 不一致的行为逻辑，除非有明确的 Rust 特有原因（如类型安全改进）

### 对应关系

| Rust Provider | pi-mono 参考 |
|---|---|
| `providers/openai_completions.rs` | `providers/openai-completions.ts` |
| `providers/anthropic.rs` | `providers/anthropic.ts` |
| `providers/google.rs` | `providers/google.ts` |
| `providers/openai_responses.rs` | `providers/openai-responses.ts` |
| `types.rs` | `providers/types.ts` + `stream.ts` |
| `stream.rs` | `stream.ts` (SSE 解析部分) |

## Git 工作流

- `dev` — 日常开发
- `main` — 发版
- Conventional Commits: `feat(scope): description`
- scope: `cli`, `sandbox`, `protocol`, `guest`, `runner`, `runtime`

## 发布 & 本地更新流程

详见 [`docs/release-process.md`](docs/release-process.md)，包含完整流程、已知坑和检查清单。

**核心原则**：一个版本号 = 一次构建 = 一个 SHA256。**绝不 force-push 已发布的 tag。**

```bash
# 标准发版三步
git checkout main && git merge dev && git push origin main
git tag vx.y.z && git push origin vx.y.z
# 等 Actions 全绿后：
brew upgrade chasey-myagi/tap/sage
```

## Agent 模型分工（多 agent 并行开发）

当本仓库用多 agent 并行做 TDD（test-writer → test-review → implementer →
Linus / code-review）时，按角色挑模型：

| 角色 | 模型 | 理由 |
|------|------|------|
| **Implementer**（让测试变绿、bug-fix、机械翻译） | **Sonnet 4.6** | 成本低、吞吐高；"按规格做"的工作不需要 meta 推理 |
| **Test-writer**（写测试 + 最小桩） | Sonnet 4.6 | 接近 implementer 工作性质 |
| **Test-review**（TDD 质量门） | **Opus 4.7** | 需要独立判断测试覆盖是否真的锁死规格 |
| **Linus-review**（锐评 / 品味审查） | Opus 4.7 | 需要识别架构异味与 race 隐患 |
| **Code-review**（工程规范质量门） | Opus 4.7 | 需要跨文件推理、安全/性能判断 |

调度器（主 agent）调用 `Agent(model: "sonnet" / "opus", ...)` 显式指定。
默认继承父 agent 模型，所以要**显式传 model 参数**才能省钱 + 保质。
