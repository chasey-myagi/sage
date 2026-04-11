# Agent Caster

Agent OS 的 Executor Caster —— 在 microVM 沙箱中安全执行 AI Agent 任务。

基于 [msb_krun](https://crates.io/crates/msb_krun) (纯 Rust microVM) 构建硬件级隔离沙箱，通过 [Rune Runtime](https://github.com/chasey-myagi/rune) 注册为分布式 Caster。

## 架构

```
Rune Runtime (gRPC :50070)
       │
       ▼
Agent Caster (crates/caster)
  ├─ 注册 agents.execute rune
  ├─ 接收 { config: AgentConfig, message: string }
  ├─ 根据 config.tools 生成 ToolPolicy
  │
  ▼
Sandbox (crates/sandbox)
  ├─ msb_krun VmBuilder → 创建 microVM
  ├─ 挂载 allowed_paths 为 Volume
  ├─ AgentRelay ← virtio-console → Guest Agent
  │
  ▼
Guest Agent (crates/guest-agent)        ← 运行在 VM 内部
  ├─ PID 1 init (mount, network)
  ├─ 接收 ExecRequest / FsRequest
  ├─ 执行命令 (白名单校验)
  └─ 返回结果
```

## 项目结构

```
agent-caster/
├── Cargo.toml                   # Workspace
├── crates/
│   ├── caster/                  # Rune Caster 入口
│   ├── sandbox/                 # Host 端沙箱 SDK (msb_krun)
│   ├── protocol/                # Host↔Guest 线协议 (CBOR)
│   ├── guest-agent/             # Guest Agent (VM 内 PID 1)
│   └── runner/                  # AgentConfig + ToolPolicy
├── configs/
│   └── feishu-assistant.yaml    # 示例 Agent 配置
└── docs/
    └── scope/
```

## 前置条件

- Rust 1.85+
- [Rune Runtime](https://github.com/chasey-myagi/rune)
- macOS Apple Silicon 或 Linux (KVM)
- `ANTHROPIC_API_KEY` 环境变量

## Quick Start

```bash
cargo build
cargo run -p agent-caster -- --runtime localhost:50070
```

## 开发

```bash
cargo build
cargo test
cargo clippy
```

详见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## License

Private.
