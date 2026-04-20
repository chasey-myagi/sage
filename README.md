# Sage

> A Rust implementation of a multi-provider AI coding agent with a full-featured terminal UI.

Sage is a terminal-native coding agent built around a clean, layered architecture:
**ai → agent-core → tui / coding-agent**. It supports streaming responses, tool use,
context compaction, and a rich TUI with Markdown rendering, fuzzy search, and image display.

---

## Architecture

```
crates/
├── ai/            # LLM provider abstraction — Anthropic, OpenAI, Qwen, Google, Bedrock…
├── agent-core/    # Agent loop, tool execution, compaction, system prompt
├── tui/           # Terminal UI component library (ratatui-based)
└── coding-agent/  # CLI entry point, session management, built-in tools
```

The four-crate structure keeps concerns separated: `ai` knows nothing about tools,
`agent-core` knows nothing about rendering, and `tui` has no LLM dependency.

---

## Quick Start

```bash
# Build (Rust 1.85+)
cargo build --release

# Run with Anthropic
export ANTHROPIC_API_KEY=sk-ant-...
./target/release/sage -p "explain this codebase"

# Run with Qwen
export DASHSCOPE_API_KEY=sk-...
./target/release/sage --provider qwen --model qwen3-235b-a22b -p "你好"

# Interactive mode
./target/release/sage
```

---

## Supported Providers

| Provider | Env Var |
|---|---|
| Anthropic | `ANTHROPIC_API_KEY` |
| OpenAI | `OPENAI_API_KEY` |
| Qwen (DashScope) | `DASHSCOPE_API_KEY` |
| Google Gemini | `GEMINI_API_KEY` |
| Moonshot (Kimi) | `MOONSHOT_API_KEY` |
| AWS Bedrock | AWS credential chain |
| Azure OpenAI | `AZURE_OPENAI_API_KEY` |
| GitHub Copilot | OAuth (browser flow) |
| … | see `crates/ai/src/provider_specs.rs` |

---

## Features

- **Streaming** — token-by-token output with SSE parsing
- **Tool use** — Read, Write, Edit, Bash, Grep, Find, LS, Glob
- **Context compaction** — proactive + reactive summarization to stay within context limits
- **Markdown rendering** — full CommonMark via pulldown-cmark with tables, code blocks, lists
- **Image support** — Kitty/iTerm2 inline image protocol, clipboard paste (macOS/WSL)
- **Fuzzy search** — slash-command autocomplete with fuzzy matching
- **Kill ring** — Emacs-style yank/kill text buffer
- **Cancellable operations** — Escape key cancels in-flight requests via `CancellationToken`
- **Settings UI** — event-driven settings list with submenu support

---

## Development

```bash
# Run all tests
cargo test

# Run TUI tests
cargo test -p tui

# Regenerate models list from models.dev
cargo run -p ai --bin generate-models -- --write
```

---

## Acknowledgements

Sage's architecture, provider implementations, and agent loop design draw heavily from
[**pi-mono**](https://github.com/elie222/pi-mono) — an open-source multi-provider AI
assistant framework. The overall package structure (ai / agent-core / tui / coding-agent),
streaming SSE handling, tool schema design, compaction logic, and TUI component system
are all shaped by pi-mono's thoughtful TypeScript design. We are grateful for the
excellent foundation it provided.

---

## License

MIT
