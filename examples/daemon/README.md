# Daemon Mode

Sage daemon runs an agent as a persistent background process reachable over a Unix socket (`~/.sage/daemon.sock`). Multiple clients can connect and send messages without restarting the agent.

## Workflow

```bash
# 1. Start the daemon with a config
sage start --config path/to/config.yaml

# 2. Send a one-shot message and print the reply
sage send "summarise the last 10 git commits"

# 3. Attach an interactive TUI session
sage connect

# 4. Stop the daemon when done
sage stop
```

## Notes

- `sage start` returns immediately; the daemon continues in the background.
- `sage connect` streams agent events live; press `Ctrl-C` to detach without stopping the daemon.
- The socket path can be overridden with `--socket <path>` on all daemon subcommands.
- Logs are written to `~/.sage/daemon.log` by default.
- Only one daemon per socket path can run at a time. `sage start` will error if a daemon is already running.
