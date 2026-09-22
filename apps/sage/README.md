# sage

one-shot CLI。`node apps/sage/bin/sage.mjs -p "任务"`（或 `sage "任务"`）在当前目录跑完一轮，把 `RunSummary` 打成一行 JSON。

```
usage: sage [-p | --print] <task> [--provider <preset>] [--model <id>] [--base-url <url>] [--api-key-env <NAME>] [--ladder provider/model,provider/model] [--budget <tokens>] [--passthrough] [--scorer rule|model|jev] [--json]
       sage stats <session.jsonl>
       sage bench run --tasks <file> --policy passthrough|compiler --model <id> [--provider <preset>] [--budget <tokens>] --out <dir>
       sage bench report <runA> <runB>
```

`--budget <tokens>` 覆盖默认预算 40000，必须是正整数，否则退出 2，stderr 一行说明。和 `--passthrough` 一起用时忽略这个值，并在 stderr 提示一句。对照实验请显式压预算，默认 40000 在短任务上不会触发编译器。

## Model Experience

### What the model sees

argv 和 stderr 进度不是模型可见面。本壳选定的 system prompt 和七个工具 schema 会进入请求；prompt 在 hash 变化时写入 `request_header`。进度行只在 stderr：`turn N · tokens in/out · tool calls`。

### Token effect

system prompt 是固定的几行，同一轮里 head 不变时后续 `request_header` 只存 hash。工具页的长度由 `@sage/tools` 的双限位和 spill 卡住。

### KV cache effect

同一次运行里 system prompt 与工具 schema 不改，head 前缀保持稳定。工具结果接在前缀之后。

## 路由

默认梯子只有当前型号。多家 key 都在时，阶段重开不会改跑更便宜的型号。要跨型号，显式传 `--ladder provider/model,provider/model`，在这些档里选目录 input 价最低、且这次视图允许的那一档。新视图碰到 `.env` 或 `secrets` 时，非当前 provider 的档会被拿掉。`--help` 里有同一行说明。

## `--provider`

`--provider <name>` 选用预设。`--base-url` 覆盖该预设的基址，`--api-key-env` 覆盖读哪一个环境变量。没拿到 key 时退出 2，stderr 点名这个变量。没写 `--provider` 时，按 deepseek、dashscope、moonshot、openai、anthropic 取第一个已经设了 key 的预设；一个都没有时退出 1。显式 `--provider` 盖住这个顺序。不写 `--model` 时用该 provider 目录的第一条：deepseek 是 `deepseek-v4-pro`，`deepseek-flash` 用 `--model` 选。每个已经设了 key 的预设都会装上适配器：OpenAI 兼容的几家是同一个适配器类，基址不同。

| provider | base URL | 环境变量 | api |
| --- | --- | --- | --- |
| openai | `https://api.openai.com/v1` | `OPENAI_API_KEY` | openai-chat |
| deepseek | `https://api.deepseek.com` | `DEEPSEEK_API_KEY` | openai-chat |
| dashscope | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `DASHSCOPE_API_KEY` | openai-chat |
| moonshot | `https://api.moonshot.cn/v1` | `MOONSHOT_API_KEY` | openai-chat |
| anthropic | `https://api.anthropic.com` | `ANTHROPIC_API_KEY` | anthropic-messages |
