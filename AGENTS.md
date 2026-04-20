# Sage — Agent 开发指南

本文档面向在 sage workspace 中使用 AI agent（Claude Code、Codex 等）协助开发的场景。

## 项目结构

```
crates/
├── ai/           # LLM providers、streaming、类型系统
├── agent-core/   # Agent 循环、tool 执行、compaction
├── tui/          # 终端 UI 组件（与 ai/agent-core 完全解耦）
└── coding-agent/ # CLI 入口、session 管理、内置工具
```

依赖方向：`ai` ← `agent-core` ← `coding-agent`，`tui` 独立。

## 代码对齐原则

`crates/ai/src/providers/` 中的 LLM Provider 代码**严格对齐 pi-mono 的 TypeScript 参考实现**：

| Rust | pi-mono 参考 |
|------|-------------|
| `providers/anthropic.rs` | `providers/anthropic.ts` |
| `providers/openai_completions.rs` | `providers/openai-completions.ts` |
| `providers/openai_responses.rs` | `providers/openai-responses.ts` |
| `providers/google.rs` | `providers/google.ts` |
| `types.rs` / `stream.rs` | `types.ts` / `stream.ts` |

修改 provider 前，先读对应的 `.ts` 文件确认逻辑一致。不要自行发明与 pi-mono 不一致的行为，除非有明确的 Rust 特有原因。

## 构建与测试

```bash
cargo build                    # 全量构建
cargo test                     # 全量测试
cargo build -p ai              # 单 crate
cargo clippy --workspace --all-targets -- -Dwarnings
cargo fmt --all
```

## CI 规范

CI 运行三步：`cargo fmt --check` → `cargo clippy -Dwarnings` → `cargo build + test`

本地提交前务必：
```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -Dwarnings
```

## 多 Agent 并行开发

在本仓库做 TDD（test-writer → implementer → reviewer）时，按角色选模型：

| 角色 | 模型 | 理由 |
|------|------|------|
| Implementer / Test-writer | Sonnet 4.6 | 成本低、吞吐高 |
| Test-review / Code-review / Linus-review | Opus 4.7 | 需要独立判断和跨文件推理 |

调度器用 `Agent(model: "sonnet" / "opus", ...)` 显式指定，不要依赖继承。

## Commit 规范

Conventional Commits，scope 用 crate 名：

```
feat(ai): add Mistral provider
fix(agent-core): handle empty tool call arguments
refactor(tui): replace hand-written markdown parser with pulldown-cmark
chore(ci): pin stable Rust toolchain
```
