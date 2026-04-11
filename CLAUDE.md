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

## Git 工作流

- `dev` — 日常开发
- `main` — 发版
- Conventional Commits: `feat(scope): description`
- scope: `caster`, `sandbox`, `protocol`, `guest`, `runner`
