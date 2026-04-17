# Sage

Sage is an embeddable AI agent execution engine with CLI, daemon mode, TUI, and trigger scheduler. It runs agents defined by YAML configs against configurable LLM providers and tool sets.

## Quick Start

```bash
# Install (from source)
cargo install --path crates/sage-cli

# Initialise a config
sage init --name my-agent > config.yaml

# Chat
sage run --config config.yaml --message "hello"
```

## Commands

| Command | Description |
|---------|-------------|
| `sage run` | Run a single agent turn (non-interactive) |
| `sage chat` | Interactive TUI chat session |
| `sage start` | Start daemon (background agent socket) |
| `sage connect` | Attach TUI to a running daemon |
| `sage send` | Send a message to a running daemon |
| `sage stop` | Shut down the daemon |
| `sage init` | Scaffold a minimal config file |
| `sage serve` | Register as a Rune Runtime Caster |

## Config Format

```yaml
name: my-agent
system_prompt: "You are a helpful assistant."
llm:
  provider: anthropic          # anthropic | openai | google | bedrock
  model: claude-haiku-4-5-20251001
  max_tokens: 4096
constraints:
  max_turns: 10
  timeout_secs: 120
tools:
  toolset: coding              # coding | web | none | custom
hooks:
  pre_tool_use:
    - command: 'echo "tool: $SAGE_TOOL_NAME" >&2'
  stop:
    - command: './evals/check_output.sh'
      timeout_secs: 10
```

Required environment variables depend on provider:

| Provider | Variable |
|----------|----------|
| `anthropic` | `ANTHROPIC_API_KEY` |
| `openai` | `OPENAI_API_KEY` |
| `google` | `GOOGLE_API_KEY` |
| `bedrock` | `AWS_*` (standard AWS credentials) |

## Architecture

```
sage-cli          CLI entry point (clap), daemon socket, TUI
     │
sage-runner       AgentConfig (YAML), ToolPolicy, hooks, channel adapters
     │
sage-runtime      Agent loop, LLM providers, tool dispatch, event stream
     │
sage-sandbox      Host-side microVM SDK (msb_krun)
     │   virtio-console
sage-guest        Guest agent — PID 1 inside VM, executes tool commands
sage-protocol     Host↔Guest wire protocol (length-prefixed CBOR)
```

Daemon mode exposes a Unix socket (`~/.sage/daemon.sock`). `sage connect` / `sage send` communicate over that socket. The trigger scheduler runs periodic agents via cron-style expressions in config.

## Building from Source

```bash
# Prerequisites: Rust 1.85+, macOS Apple Silicon or Linux (KVM)
cargo build --workspace
cargo build -p sage-cli --release

# Run tests
cargo test --workspace

# Lint
cargo clippy --workspace -- -D warnings
```

## License

Private.
