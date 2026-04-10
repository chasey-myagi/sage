# Agent Caster

Agent OS 的 Executor Caster —— 基于 [pi-mono](https://github.com/nicepkg/pi-mono) 构建的 Agent 执行器，通过 [Rune Runtime](https://github.com/chasey-myagi/rune) 注册为分布式 Caster。

接收 Agent 配置 + 用户消息，使用 pi-mono 的 Agent 内核执行 LLM agent loop，返回结果。

## 架构

```
Rune Runtime
    │ gRPC
    ▼
Agent Caster (本项目)
    │
    ├─ 注册 agents.execute rune
    ├─ 接收 { config, message }
    ├─ 构建 pi-mono Agent (LLM + tools)
    └─ 执行 agent loop → 返回结果
```

## 前置条件

- Node.js >= 20
- [Rune Runtime](https://github.com/chasey-myagi/rune) 运行中
- `ANTHROPIC_API_KEY` 环境变量

## Quick Start

```bash
# 安装依赖
pnpm install

# 启动（连接本地 Rune Runtime）
pnpm start --runtime localhost:50070

# 开发模式
pnpm dev --runtime localhost:50070
```

## 开发

```bash
pnpm install
pnpm build
pnpm test
pnpm lint
```

详见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## License

Private.
