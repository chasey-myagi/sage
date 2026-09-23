# @sage/core

内核：只追加的 Log，以及每次调用模型前的 View seam。

## 轮数上限

`AgentSession.run` 不传 `maxTurns` 时，上限是 80。适配器一直以 `tool_use` 结束时，第 80 轮结束后追加 `terminal`，`outcome` 为 `max_turns`，不抛。调用方传入的正整数覆盖这个默认值。

## Model Experience

### What the model sees

head + messages 全由 ViewProvider 决定，内核只断言与落盘。

### Token effect

内核不增删任何 token。

### KV cache effect

内核保证阶段内 head 逐字节不变、断点原样透传，`request_header` 是事后证据。换模型不改 head。

## model_change

阶段边界如果视图带了 `epochStart.model`，并且和当前型号不同，内核在 `view_epoch` 之前追加 `model_change`。payload 是 `{ from, to, reason }`。reason 是 `route:` 加上清单 `epochDecision.reason` 里 `;route:` 之后的那段。然后切到 `adapters[provider]`，用新型号再 `prepare` 一次。随后的 `request_header` 用这次 prepare 的 provider、model 和 adapterDefaults。

找不到该 provider 的适配器时，追加 `terminal`，outcome 为 `error`，不发请求。型号相同，或者没有 `epochStart.model`，不写 `model_change`。

`RunSummary.modelChanges` 等于这次 run 里追加的 `model_change` 条数。

## 健康度

一次 run 结束时，`RunSummary.health` 和 `terminal` entry 的 `health` 是同一份 `runHealth(path)`。三个数都从 Log 里已经有的 entry 算出来。

| 数 | 口径 |
| --- | --- |
| token 总量 | `tokens.total` = input + cacheRead + cacheCreation + output。`input` 是各条 assistant `usage.inputOther` 之和，另外三项分别是 `inputCacheRead`、`inputCacheCreation`、`output`。每个 resolved 的 token 由基准层拿这个总量去除，本包只给这一次 run 的总量。 |
| 缓存命中率 | `cacheHitRatio` = Σ cacheRead / Σ (cacheRead + inputOther + cacheCreation)。分子分母都按 token 加总，不是把每一轮的比率再平均。没有 assistant usage，或分母为 0 时，值为 null。 |
| expand 率 | 最后一条 manifest 的 `budget.expands / budget.omitted`。这两个字段已经是累计值，只读最后一条。`omitted` 为 0 时值为 null。 |
| 子任务 | `subtasks.count` 与 `subtasks.tokens` 汇总父 Log 里 `spawn_subtask` / `spawn_subtasks` 的 tool result details。`cacheHitRatio` 和 `expandRate` 不含这些 token。 |

`turns` 是 assistant 消息条数。`epochs` 是最大 `view_epoch.epoch` 加 1，没有 `view_epoch` 时为 1。`modelChanges` 是路径上的 `model_change` 条数。`scorerCalls` 与 `scorerFallbacks` 把每条 manifest 的 `scorer.calls` 和 `scorer.fallback` 加起来；`fallback: true` 计 1。`shadow.calls` 数带 `shadow` 对象的清单条数。`shadow.usage` 把这些对象里的 `usage` 四元组加起来：`input` 是 `inputOther`，另外三项是 `inputCacheRead`、`inputCacheCreation`、`output`。

`health` 里只有这些数字和 null。块原文、claim 文本、路径、子会话 id 都不进这个对象。离线看同一份数用 `sage stats <session.jsonl>`。子 usage 只出现在 `subtasks`，不进上面的 token 总量和两个比率。

构造可以传 `adapters`（provider 到适配器），也可以继续传单个 `adapter`。单个适配器记在起始型号的 provider 名下。

## 测试

无 key 是默认：测试进程会剥掉 `*_API_KEY`，并用录制的响应回放，不打真网。
`pnpm test:update` 重写期望产物，PR 里要逐行看这些 diff。
破坏前缀稳定会炸快照：head 变了，下一轮就不再是 `<last>` 加增量，而是整段重印。
