# Sage

> 领域专家 Agent 框架。给每个领域配一个独立的 agent，它有自己的工作空间、记忆、
> 和可按需调用的技能库；多轮对话在本地持续演化。

Sage 不是通用 chatbot — 它的立意是**一个 agent 一个领域**：飞书运维有一个
agent，知识库管理有一个 agent，代码审查有一个 agent。每个 agent 自己维护
它的 skill 库（安装、评估、重写），用户说最少的话，agent 完成最精准的事。

---

## Quick Start

```bash
# Install (from source, Rust 1.85+)
cargo install --path crates/sage-cli

# 1. 初始化一个 agent（创建 ~/.sage/agents/<name>/ 骨架）
sage init --agent feishu --provider kimi --model moonshot-v1-auto

# 2. 装 skill（local 目录 或 git URL）
sage skill add --agent feishu ~/.claude/skills/lark-base
sage skill add --agent feishu https://github.com/your-org/custom-skill.git

# 3. 对话（--dev 跳过 microVM，host 模式直接跑）
sage chat --agent feishu --dev

# 或者作为 daemon 后台驻留
sage start --agent feishu
sage connect --agent feishu
sage stop --agent feishu
```

---

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│  sage-cli         CLI + daemon + chat TUI + trigger scheduler    │
│  sage-runner      AgentConfig (YAML) / ToolPolicy / hook scripts │
│  sage-runtime     Agent loop · LLM providers · tool dispatch     │
│  sage-sandbox     Host-side microVM SDK (msb_krun / libkrunfw)   │
│  sage-guest       VM-内 PID 1 — 执行 tool 命令                    │
│  sage-protocol    Host ↔ Guest wire (length-prefixed CBOR)       │
└──────────────────────────────────────────────────────────────────┘
```

一个 agent 的磁盘布局：

```
~/.sage/agents/<name>/
├── AGENT.md                # 一句话指令：去哪儿找 skill
├── config.yaml             # provider / model / sandbox / memory
├── memory/
│   └── MEMORY.md           # 跨 session 的沉淀
└── workspace/              # mount 到 /workspace（sandbox 模式下）
    ├── SCHEMA.md
    ├── wiki/               # 知识库 pages
    ├── raw/sessions/       # 对话原始记录
    ├── metrics/            # 每 task 一份 JSON + summary.json
    └── skills/
        ├── INDEX.md        # agent 自维护的技能索引
        ├── lark-base/
        │   └── SKILL.md
        ├── lark-calendar/
        │   ├── SKILL.md
        │   └── references/
        └── …
```

---

## Skills 模型

**Agent 自主发现、按需加载**：system prompt 不预装任何 skill body；agent 拿到
任务后读 `workspace/skills/INDEX.md`（自维护的目录），再用 `Read` 工具取
`<name>/SKILL.md` 详细内容。token 成本按需，不按安装数量线性叠加。

### 安装

```bash
# local path（绝对 / ~ / ./ 开头）
sage skill add --agent feishu ~/Dev/skills/my-skill
sage skill add --agent feishu /abs/path/to/skill --name alias

# git URL（https:// / git@ / ssh:// / .git 结尾）
sage skill add --agent feishu https://github.com/u/skill.git
sage skill add --agent feishu git@github.com:u/skill.git
```

约定：skill 目录根必须有 `SKILL.md`（带 YAML frontmatter 的 `description:`
用于填 `INDEX.md` 的一行）。其它子目录（`references/` / `scenes/` 等）
随源原样拷贝，agent 按需 Read。

### 查看 / 评分

```bash
sage skill list --agent feishu
sage skill-score --agent feishu                    # 用 token 效率打分
sage skill-score --agent feishu --needs-evaluation # 低分 + 足够用量的筛出来
```

score = `tokens_best / tokens_avg`（≤ 1.0，越高越好）。达到阈值的 skill
会在未来被 `SkillEvaluation` session 自动重写（v0.0.3 task）。

---

## Supported LLM Providers

18 个内置 provider；多 API family 共存。

| Provider id | API kind | Env var |
|---|---|---|
| `anthropic` / `minimax` | Anthropic Messages | `ANTHROPIC_API_KEY` / `MINIMAX_API_KEY` |
| `openai` / `azure-openai` | OpenAI Responses | `OPENAI_API_KEY` / `AZURE_OPENAI_API_KEY` |
| `openai-compat` / `kimi` / `qwen` / `deepseek` / `zai` / `bytedance-ark` / `ollama` / `vllm` | OpenAI Completions | provider-specific (`MOONSHOT_API_KEY` / `DASHSCOPE_API_KEY` / …) |
| `google` / `google-vertex` | Gemini | `GEMINI_API_KEY` / Vertex SA |
| `bedrock` | AWS Bedrock | standard AWS creds |
| `github-copilot` / `openrouter` | via OpenAI-compat | `GITHUB_TOKEN` / `OPENROUTER_API_KEY` |

`sage validate --agent <name>` 会校验 config.yaml 里的 provider 是否在这
张表里。未知 provider 启动前就报错，不等到 LLM 调用时才炸。

---

## Config Reference

```yaml
name: feishu
description: "飞书面试管理 Agent"
llm:
  provider: kimi
  model: moonshot-v1-auto
  max_tokens: 4096          # optional，留空走 ProviderSpec 默认
  context_window: 131072    # optional
system_prompt: "你是飞书专员。"
tools:
  toolset: coding            # coding | ops | web | minimal | readonly
constraints:
  max_turns: 20
  timeout_secs: 300
sandbox:
  mode: host                 # host | microvm (microvm 需要 libkrunfw)
  workspace_host: ~/.sage/agents/feishu/workspace
memory:
  inject_as: prepend_system
  auto_load:
    - AGENT.md
    - memory/MEMORY.md
```

`sandbox.mode: host` 或 CLI `--dev` flag 都会跳过 microVM，直接在 host
fs 上执行 tool。没装 libkrunfw 的机器必走这条路径。

---

## Observability

每次 `sage chat` session 结束后，`UserDriven` session 会落盘：
- `workspace/metrics/<ulid>.json` — 完整 TaskRecord（tokens / turn_count /
  tool_call_count / success / crafts_active / …）
- `workspace/metrics/summary.json` — 滚动最近 50 条

跟踪：
```bash
# 单次 session 的 tracing 输出
SAGE_LOG=debug sage chat --agent feishu --dev

# JSON 日志（配合 jq / lnav 消费）
SAGE_LOG_FORMAT=json sage chat --agent feishu --dev
```

---

## Building from Source

```bash
# Prerequisites: Rust 1.85+, macOS Apple Silicon 或 Linux (KVM for microvm)
cargo build --workspace
cargo build -p sage-cli --release

# Tests
cargo test --workspace

# Lint
cargo clippy --workspace -- -D warnings
```

---

## Status

Pre-release（v0.0.x）。API / CLI 命令 / 磁盘布局可能在 v0.1.0 前变动一次。
生产部署请等 v0.1.0。

- Changelog: [CHANGELOG.md](./CHANGELOG.md)
- 开发规划：[docs/TODO.md](./docs/TODO.md)

## License

Private.
