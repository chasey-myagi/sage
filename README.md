# Sage

> 领域专家 Agent 应用 — 给一个领域配置一个有独立工作空间、有记忆沉淀的专员。

Sage 是 terminal-native agent，构建在分层架构上：**ai → agent-core → tui / sage-cli**。
支持流式响应、工具调用、上下文压缩；TUI 含 Markdown 渲染、approval 对话、permission 模式。

---

## Architecture

```
crates/
├── ai/        # LLM provider 抽象 — Anthropic, OpenAI-compat, Google
├── agent-core/  # Agent loop / 工具执行 / 压缩 / MCP / hook
├── tui/         # ratatui 组件库（input / markdown / approval）
└── sage-cli/    # CLI 入口（bin name = sage）+ session 管理
```

四 crate 分层：`ai` 不知道 tool 的存在；`agent-core` 不知道渲染；`tui` 不依赖 LLM。

---

## Quick Start

```bash
# Build (Rust 1.85+)
cargo build --release

# Anthropic
export ANTHROPIC_API_KEY=sk-ant-...
./target/release/sage -p "explain this codebase"

# Qwen / Kimi / OpenAI-compat 走同一个 provider
export DASHSCOPE_API_KEY=sk-...
./target/release/sage --provider qwen --model qwen3-235b-a22b -p "你好"

# 交互模式
./target/release/sage
```

---

## Supported Providers

3 个内置：

| Provider | Env Var | API |
|---|---|---|
| Anthropic | `ANTHROPIC_API_KEY` | anthropic-messages |
| Google Gemini | `GEMINI_API_KEY` | google-generative-ai |
| OpenAI-compat | `OPENAI_API_KEY` / `DASHSCOPE_API_KEY` / `MOONSHOT_API_KEY` / `DEEPSEEK_API_KEY` / … | openai-completions |

所有 OpenAI 兼容端点（Qwen / Kimi / DeepSeek / Groq / xAI / OpenRouter / Cerebras …）走同一个 provider，按 `--provider <name>` 自动选 base_url + env var。

---

## Features

- **流式响应** — SSE token-by-token
- **工具集** — Read / Write / Edit / Bash / Grep / Find / LS / web_fetch / web_search + MCP
- **上下文压缩** — 主动 + 被动两策略
- **Markdown 渲染** — CommonMark via pulldown-cmark
- **Approval 对话** — 工具执行前可显式批准/拒绝
- **Permission 模式** — `--permission-mode` 控制工具白名单严格度
- **可中断** — Esc 取消进行中的请求

---

## Development

```bash
# 全量测试
cargo test --workspace

# TUI 测试单独
cargo test -p tui
```

多 worktree 并行开发时**必须**用隔离 cargo target：

```bash
CARGO_TARGET_DIR=./target cargo build --workspace
```

详见 `CLAUDE.md` 末段。

---

## Acknowledgements

Sage 的架构、provider 实现和 agent loop 设计直接借鉴自 [**pi-mono**](https://github.com/badlogic/pi-mono) — 一个开源的多 provider AI assistant 框架。整体包结构（ai / agent-core / tui / sage-cli）、SSE 流式处理、工具 schema 设计、compaction 逻辑、TUI 组件系统都由 pi-mono 的 TypeScript 设计塑造。感谢这份扎实的基础。

记忆模型设计借鉴 [**GenericAgent**](https://github.com/lsdefine/GenericAgent) 的 L0–L4 分层。

---

## License

MIT
