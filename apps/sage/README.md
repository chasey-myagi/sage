# sage

one-shot CLI。`node apps/sage/bin/sage.mjs -p "任务"`（或 `sage "任务"`）在当前目录跑完一轮，把 `RunSummary` 打成一行 JSON。

## Model Experience

### What the model sees

argv 和 stderr 进度不是模型可见面。本壳选定的 system prompt 和七个工具 schema 会进入请求；prompt 在 hash 变化时写入 `request_header`。进度行只在 stderr：`turn N · tokens in/out · tool calls`。

### Token effect

system prompt 是固定的几行，同一轮里 head 不变时后续 `request_header` 只存 hash。工具页的长度由 `@sage/tools` 的双限位和 spill 卡住。

### KV cache effect

同一次运行里 system prompt 与工具 schema 不改，head 前缀保持稳定。工具结果接在前缀之后。
