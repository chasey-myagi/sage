# agent-caster

Agent OS 的 Executor Caster — Rust workspace，基于 msb_krun microVM 沙箱。

## 项目结构

```
crates/
├── caster/          # Rune Caster 入口 (clap CLI + Rune SDK)
│   └── src/
│       ├── main.rs  # CLI 解析 + 启动
│       └── serve.rs # Rune 注册 + 任务处理
├── sandbox/         # Host 端沙箱 SDK
│   └── src/
│       ├── builder.rs   # SandboxBuilder (VM 配置)
│       ├── handle.rs    # SandboxHandle (exec/fs 操作)
│       ├── relay.rs     # AgentRelay (virtio-console 通信)
│       └── error.rs
├── protocol/        # Host↔Guest 线协议
│   └── src/
│       ├── messages.rs  # HostMessage / GuestMessage
│       └── wire.rs      # 长度前缀 CBOR 编解码
├── guest-agent/     # Guest Agent (VM 内 PID 1)
│   └── src/
│       ├── main.rs  # init + 主循环
│       ├── init.rs  # mount filesystems
│       ├── exec.rs  # 命令执行
│       └── fs.rs    # 文件操作
└── runner/          # Agent 配置 + 策略
    └── src/
        ├── config.rs    # AgentConfig (YAML 解析)
        └── tools.rs     # ToolPolicy (白名单校验)
```

## Build & Run

```bash
cargo build
cargo test
cargo run -p agent-caster -- --runtime localhost:50070
```

## 依赖

- msb_krun — 纯 Rust microVM (libkrun Rust 化)
- Rune TS/Rust SDK (Phase 2)
- Rune Runtime 需先启动

## 代码对齐原则

`agent-runtime` crate 中的 LLM Provider 代码（`src/llm/providers/`、`src/llm/stream.rs`、`src/llm/types.rs` 等）**必须严格对齐 pi-mono 的 TypeScript 参考实现**。

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
- scope: `caster`, `sandbox`, `protocol`, `guest`, `runner`, `runtime`
