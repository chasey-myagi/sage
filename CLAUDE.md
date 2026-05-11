# Sage

领域专家 Agent 应用 — 给一个领域配置一个有独立工作空间、有记忆沉淀的专员。
通过 TUI 交互，工具白名单 + approval 做执行边界，未来通过 Rune Caster 调度。

## 项目结构

```
crates/
├── ai/          # LLM provider 抽象 + 流式 SSE + 类型
├── agent-core/  # Agent loop / 工具执行 / 压缩 / MCP / hook
├── tui/         # ratatui 组件库（输入框 / markdown / approval 等）
└── sage-cli/    # CLI 入口（bin name = sage）+ session 管理
```

四 crate 分层：`ai` 不知道 tool 的存在；`agent-core` 不知道渲染；`tui` 不依赖 LLM。

## Build & Run

```bash
cargo build
cargo test
cargo run -p sage-cli -- -p "hello"
cargo run -p sage-cli --release
```

CLI bin 叫 `sage`，package 叫 `sage-cli`（见 `crates/sage-cli/Cargo.toml`）。

## 代码对齐原则

`crates/ai` 的 LLM provider 代码（`providers/`、`stream.rs`、`types.rs`）**严格对齐 pi-mono 的 TypeScript 参考实现**。

参考源码位置：`~/Dev/cc/external/pi-mono/packages/ai/src/providers/`

### 对应关系

| Rust Provider | pi-mono 参考 |
|---|---|
| `providers/openai_completions.rs` | `providers/openai-completions.ts` |
| `providers/anthropic.rs` | `providers/anthropic.ts` |
| `providers/google.rs` | `providers/google.ts` |
| `types.rs` | `providers/types.ts` + `stream.ts` |
| `stream.rs` | `stream.ts` (SSE 解析部分) |

修改 provider 代码前先读对应的 pi-mono `.ts` 文件，确认逻辑一致；不要自行发明与 pi-mono 不一致的行为，除非有明确的 Rust 特有原因（如类型安全改进）。

## Git 工作流

- `dev` — 日常开发
- `main` — 发版
- Conventional Commits: `feat(scope): description`
- scope: `ai`, `agent-core`, `tui`, `cli`

## 发布 & 本地更新流程

详见 [`docs/release-process.md`](docs/release-process.md)。

**核心原则**：一个版本号 = 一次构建 = 一个 SHA256。**绝不 force-push 已发布的 tag。**

```bash
# 标准发版三步
git checkout main && git merge dev && git push origin main
git tag vx.y.z && git push origin vx.y.z
# 等 Actions 全绿后：
brew upgrade chasey-myagi/tap/sage
```

## Agent 模型分工（多 agent 并行开发）

按角色挑模型：

| 角色 | 模型 | 理由 |
|------|------|------|
| **Implementer**（让测试变绿、bug-fix、机械翻译） | **Sonnet 4.6** | 成本低、吞吐高；"按规格做"的工作不需要 meta 推理 |
| **Test-writer**（写测试 + 最小桩） | Sonnet 4.6 | 接近 implementer 工作性质 |
| **Test-review**（TDD 质量门） | **Opus 4.7** | 需要独立判断测试覆盖是否真的锁死规格 |
| **Linus-review**（锐评 / 品味审查） | Opus 4.7 | 需要识别架构异味与 race 隐患 |
| **Code-review**（工程规范质量门） | Opus 4.7 | 需要跨文件推理、安全/性能判断 |

调度器调用 `Agent(model: "sonnet" / "opus", ...)` 显式指定，否则默认继承父 agent 模型。
