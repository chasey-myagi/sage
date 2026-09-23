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
| 工具错误 | `toolErrors` 是路径上 `isError: true` 的 `tool_result` 条数。这个数不结束 run。 |
| 整轮重试 | `turnRetries` 是路径上 `turn_retry` 的条数。 |

`turns` 是 assistant 消息条数。`epochs` 是最大 `view_epoch.epoch` 加 1，没有 `view_epoch` 时为 1。`modelChanges` 是路径上的 `model_change` 条数。`scorerCalls` 与 `scorerFallbacks` 把每条 manifest 的 `scorer.calls` 和 `scorer.fallback` 加起来；`fallback: true` 计 1。`calls` 只数真正发出的打分请求，所以打分器在发请求前就放弃的那一轮计 0 次调用、1 次 fallback。`entryScored.calls` 同一口径。`shadow.calls` 数带 `shadow` 对象的清单条数。`shadow.usage` 把这些对象里的 `usage` 四元组加起来：`input` 是 `inputOther`，另外三项是 `inputCacheRead`、`inputCacheCreation`、`output`。

`health` 里只有这些数字和 null。块原文、claim 文本、路径、子会话 id 都不进这个对象。离线看同一份数用 `sage stats <session.jsonl>`。子 usage 只出现在 `subtasks`，不进上面的 token 总量和两个比率。

## 工具错误

工具永远不能杀会话。`execute` 抛出的同步异常和被拒绝的 Promise，以及包住它的 `around` 抛错，都变成一条 `tool_result`：`isError` 为 true，正文是 `<工具名>: <message>`。这条结果写进下一条用户消息，模型下一轮看得到。连续多次也一样，默认看 5 次也不会追加 `outcome: "error"` 的 terminal。`runHealth.toolErrors` 只计数。

会终止的是内核自己的失败和 provider 的失败：视图断言、没有适配器、非传输类的适配器 finish error、传输类错误在整轮重试耗尽之后，以及工具管线以外的抛错。

构造可以传 `adapters`（provider 到适配器），也可以继续传单个 `adapter`。单个适配器记在起始型号的 provider 名下。

## 整轮重试

流还没开始时的重试在 provider：`fetch` 抛错、2xx 没有响应体、HTTP 429、HTTP 5xx，最多 2 次。响应体一开始，provider 就不再重试。

流已经开始之后，如果 finish 是传输类错误，并且这一轮还没有把 assistant 消息写进 Log，core 重发同一条请求。只认 finish error 上的 `transport: true`，不看错误文案。provider 在读 body 抛错、以及 body 在收到结束信号前正常关闭时打这个标记。HTTP 4xx 不重试。最多再发 2 次，先等 2 秒，再等 8 秒。构造参数 `sleep` 可注入，测试不真等。第 3 次仍失败，才把这次的 assistant 写入，并追加 terminal，outcome 为 `error`。

每次重发之前追加 `turn_retry`，payload 是 `{ turn, attempt, reason }`。这条持久化，不进模型请求。被重发丢掉的半截文本不进 Log。

## 测试

无 key 是默认：测试进程会剥掉 `*_API_KEY`，并用录制的响应回放，不打真网。
`pnpm test:update` 重写期望产物，PR 里要逐行看这些 diff。
破坏前缀稳定会炸快照：head 变了，下一轮就不再是 `<last>` 加增量，而是整段重印。
