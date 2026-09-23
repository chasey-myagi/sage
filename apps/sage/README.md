# sage

one-shot CLI。`node apps/sage/bin/sage.mjs -p "任务"`（或 `sage "任务"`）在当前目录跑完一轮，把 `RunSummary` 打成一行 JSON。

```
usage: sage [-p | --print] <task> [--provider <preset>] [--model <id>] [--base-url <url>] [--api-key-env <NAME>] [--ladder provider/model,provider/model] [--budget <tokens>] [--max-turns <n>] [--tool-network none|full] [--passthrough] [--scorer rule|model|jev] [--shadow-scorer rule|model|jev] [--scorer-apply entry|rebuild] [--json]
       sage stats <session.jsonl>
       sage bench run --tasks <file> --policy passthrough|compiler --model <id> [--provider <preset>] [--budget <tokens>] [--max-turns <n>] [--tool-network none|full] [--scorer rule|model|jev] [--shadow-scorer rule|model|jev] [--scorer-apply entry|rebuild] --out <dir>
       sage bench report <runA> <runB>
```

`--budget <tokens>` 覆盖默认预算 40000，必须是正整数，否则退出 2，stderr 一行说明。和 `--passthrough` 一起用时忽略这个值，并在 stderr 提示一句。对照实验请显式压预算，默认 40000 在短任务上不会触发编译器。

`--max-turns <n>` 覆盖内核默认的 80 轮，必须是正整数，否则退出 2，stderr 一行说明。这个数交给 `session.run`，`--passthrough` 同样生效。`bench run` 每个任务的 argv 里都有 `--max-turns`：写了旗标用那个数，没写就是 80。`results.json` 顶层 `maxTurns` 记下这个数。

`--tool-network none|full` 默认 `full`。`none` 只切断 bash 子进程的网络，父进程照常请求模型。当前平台做不到时退出 2，stderr 一行说明，不会悄悄放开。`bench run` 的默认是 `none`。

`--shadow-scorer rule|model|jev` 给根编译器装上影子打分，默认不装。它每轮只给新进入视图的工具块打分，结果写在清单的 `shadow` 上。影子只记不生效：不改档次、冻结和预算，也不改模型看到的消息。`jev` 同样读 `TYPESAFE_API_KEY`，缺失则退出 2。`--passthrough` 不编译上下文，这个旗标不调用打分器。

`--scorer-apply entry|rebuild` 默认 `rebuild`：打分只在阶段重开时生效。`entry` 改为工具结果进入视图时就定档，前缀从第一次出现就是最终形态。DeepSeek 的缓存是前缀缓存，出窗口时再改档会把后面的近期内容每轮重新全价编码，按写入价和缓存读价要很多轮才回本。`entry` 必须同时写 `--scorer model` 或 `--scorer jev`，否则退出 2，stderr 点名 `--scorer model|jev`。规则打分不改档，入口生效没有意义。`entry` 不再另跑影子。`runHealth.entryScored` 汇总这次调用的次数、none / summary / full 和 usage。

`sage stats <session.jsonl>` 在 `runHealth` 同一行 JSON 里再给出三个对账数。`shadowNone` 是影子判过 `none` 的块 id 个数，同一个 id 只算一次。`laterNeeded` 是其中后来仍需要的个数：该块 id 出现在更后面的 `expand` 参数 `id` 里，或者该块是 `read` 的结果，且同一 `path` 在更后的轮次又被 `read` 了一次。判 `none` 的那一轮里再次 read 不算后续轮次。`precision = 1 − laterNeeded / shadowNone`。`shadowNone` 为 0 时 `precision` 是 null。这三个数是事后标签，编译器不读它们。

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
