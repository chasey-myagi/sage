# Sage

Embeddable AI Agent execution engine — 在 msb_krun microVM 沙箱中安全执行 AI Agent 任务。

基于 [msb_krun](https://crates.io/crates/msb_krun) (纯 Rust microVM) 构建硬件级隔离沙箱，通过 [Rune Runtime](https://github.com/chasey-myagi/rune) 注册为分布式 Caster。

## 架构

```
Rune Runtime (gRPC :50070)
       │
       ��
Sage CLI (crates/sage-cli)
  ├─ 注册 agents.execute rune (Phase 2)
  ├─ 接收 { config: AgentConfig, message: string }
  ├─ 根据 config.tools 生成 ToolPolicy
  │
  ▼
Sage Runtime (crates/sage-runtime)
  ├─ Agent Loop (LLM + Tools + Event Stream)
  ├─ LLM Providers (Anthropic, OpenAI, Google, etc.)
  │
  ▼
Sandbox (crates/sage-sandbox)
  ├─ msb_krun VmBuilder → 创建 microVM
  ├─ AgentRelay ← virtio-console → Guest Agent
  │
  ▼
Guest Agent (crates/sage-guest)        ← 运行在 VM 内部
  ├─ PID 1 init (mount, security)
  ├─ 接收 ExecRequest / FsRequest
  ├─ 执行命令 (白名单校验)
  └─ 返回结果
```

## ��目结构

```
sage/ (原 agent-caster)
├── Cargo.toml                   # Workspace
├── crates/
│   ├── sage-cli/                # CLI 入口 (clap + Rune SDK stub)
│   ├── sage-runtime/            # Agent 执行引擎 (LLM + Tools + Events)
│   ├── sage-sandbox/            # Host 端沙箱 SDK (msb_krun)
│   ├── sage-protocol/           # Host↔Guest 线协议 (CBOR)
│   ├── sage-guest/              # Guest Agent (VM 内 PID 1)
│   └── sage-runner/             # AgentConfig + ToolPolicy
├── configs/
│   ├── coding-assistant.yaml
│   ├── deepseek-coder.yaml
│   └── feishu-assistant.yaml
└── docs/
    └── future.md                # 定位 & 规划
```

## 前置条件

- Rust 1.85+
- [Rune Runtime](https://github.com/chasey-myagi/rune) (可选，serve 模式需要)
- macOS Apple Silicon 或 Linux (KVM)
- LLM API Key 环境变量 (如 `ANTHROPIC_API_KEY`)

## Quick Start

```bash
cargo build
cargo run -p sage-cli -- run --config configs/coding-assistant.yaml --message "hello"
```

## 开发

```bash
cargo build
cargo test --workspace
cargo clippy --workspace
```

详见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## License

Private.
