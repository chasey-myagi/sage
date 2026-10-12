# @sage/core

内核：只追加的 Log，以及每次调用模型前的 View seam。

## 落盘保证

`JsonlLogStore.append` 把整行写完才进内存（遇到短写会循环写完剩余字节），文件永远是内存的超集。写入中途失败（`writeSync` 抛错或不再前进）时，先从文件末尾截掉这一行已写出的字节再抛出原错误，内存、leaf 与文件都保持原样，之后的 append 从干净的行首开始；截断本身也失败时，该会话拒绝后续 append（错误的 `cause` 是截断失败的原因），半行留在末尾，下次用新的 store 打开时按撕裂尾行丢弃。写进去的行在 OS 页缓存里，进程被杀（超时、Ctrl-C、崩溃）不丢。store 不调用 fsync，整机断电或 OS 崩溃可能丢掉或损坏文件尾部：页缓存不保证按顺序回写，损坏如果不只在最后一行，重开时抛 `LogCorruptedError`。只有最后一行撕裂时照常丢弃并截掉。

## 轮数上限

`AgentSession.run` 不传 `maxTurns` 时，上限是 80。适配器一直以 `tool_use` 结束时，第 80 轮结束后追加 `terminal`，`outcome` 为 `max_turns`，不抛。调用方传入的正整数覆盖这个默认值。

## Model Experience

### What the model sees

head + messages 全由 ViewProvider 决定，内核只断言与落盘。

### Token effect

内核不增删任何 token。

### KV cache effect

内核保证阶段内 head 逐字节不变、断点原样透传，`request_header` 是事后证据。换模型不改 head。

## request_header

每次调模型前，内核用 `appendRequestEntries` 依次追加 `view_epoch`（需要时）、`request_header`、`manifest`。离线回放写影子 Log 也调用它，两边只有这一份实现。

`request_header` 的字段：`provider`、`model`、`sampling`（这次调用实际生效的采样参数）、`adapterDefaults`、`systemPromptHash`、`toolsHash`、`contextWindow`、`pricing`，以及按需出现的 `system` 与 `tools`。`contextWindow`（token，正的有限数）和 `pricing`（`Pricing`，目录里没有定价时为 null）是这次 `PreparedCall` 的快照，回放读它们，不查当前目录。`system` / `tools` 全文只在 hash 与路径上前一条 `request_header` 不同、或还没有前一条时写入；其余轮次只留 hash。缺 `contextWindow` 或 `pricing` 的 header 校验不通过。

读回 `request_header` 用 `requestHeaderOf(entry)`，取路径上最后一条用 `lastRequestHeader(path)`：只按 `kind` 收窄，不逐字段校验。内核 store 在追加和加载时都跑过校验器，从 `store.path` / `store.all` 拿到的 entry 形状已经成立；没经过 store 的手搭 entry 不在这个保证里。

## model_change

阶段边界如果视图带了 `epochStart.model`，并且和当前型号不同，内核在 `view_epoch` 之前追加 `model_change`。payload 是 `{ from, to, reason }`。reason 逐字取自 `epochStart.routeReason`，内核不解析也不拼接；`model` 与 `routeReason` 要么同时给出，要么都不给。然后切到 `adapters[provider]`，用新型号再 `prepare` 一次。随后的 `request_header` 用这次 prepare 的 provider、model 和 adapterDefaults。

找不到该 provider 的适配器时，追加 `terminal`，outcome 为 `error`，不发请求。型号相同，或者没有 `epochStart.model`，不写 `model_change`。

`RunSummary.modelChanges` 等于这次 run 里追加的 `model_change` 条数。

## 健康度

一次 run 结束时，`RunSummary.health` 和 `terminal` entry 的 `health` 是同一份 `runHealth(path)`。三个数都从 Log 里已经有的 entry 算出来。

| 数 | 口径 |
| --- | --- |
| token 总量 | `tokens.total` = input + cacheRead + cacheCreation + output。`input` 是各条 assistant `usage.inputOther` 之和，另外三项分别是 `inputCacheRead`、`inputCacheCreation`、`output`。每个 resolved 的 token 由基准层拿这个总量去除，本包只给这一次 run 的总量。 |
| 缓存命中率 | `cacheHitRatio` = Σ cacheRead / Σ (cacheRead + inputOther + cacheCreation)。分子分母都按 token 加总，不是把每一轮的比率再平均。没有 assistant usage，或分母为 0 时，值为 null。 |
| expand 率 | 最后一条 manifest 的 `budget.expands / budget.omitted`。这两个字段已经是累计值，只读最后一条。`omitted` 为 0 时值为 null。 |
| 子任务 | `subtasks.count` 与 `subtasks.tokens` 汇总父 Log 里任何工具在 tool result details 上报告的 `childRuns`（`ChildRun[]`，一项一个子会话，每项带 `usage`）。内核不按工具名识别子会话。`cacheHitRatio` 和 `expandRate` 不含这些 token。 |
| 工具错误 | `toolErrors` 是路径上 `isError: true` 的 `tool_result` 条数。这个数不结束 run。 |
| 整轮重试 | `turnRetries` 是路径上 `turn_retry` 的条数。 |

`turns` 是 assistant 消息条数。`epochs` 是最大 `view_epoch.epoch` 加 1，没有 `view_epoch` 时为 1。`modelChanges` 是路径上的 `model_change` 条数。`scorerCalls` 与 `scorerFallbacks` 把每条 manifest 的 `scorer.calls` 和 `scorer.fallback` 加起来；`fallback: true` 计 1。`calls` 只数真正发出的打分请求，所以打分器在发请求前就放弃的那一轮计 0 次调用、1 次 fallback。`scorerFailedRequests`、`entryScored.failedRequests`、`shadow.failedRequests` 分别加总三种报告里的 `failedRequests`：同一次打分里其他请求答上了、自己失败的请求数，缺这个字段的清单加 0。`entryScored.calls` 与 `shadow.calls` 同一口径：`shadow.calls` 加总各清单 `shadow.calls`，影子打分缺 key 这类没发请求的轮次计 0。`shadow` 对象必须带 `calls`，缺了校验不通过。`shadow.usage` 把各 `shadow` 对象里的 `usage` 四元组加起来：`input` 是 `inputOther`，另外三项是 `inputCacheRead`、`inputCacheCreation`、`output`。

`health` 里只有这些数字和 null。块原文、claim 文本、路径、子会话 id 都不进这个对象。离线看同一份数用 `sage stats <session.jsonl>`。子 usage 只出现在 `subtasks`，不进上面的 token 总量和两个比率。

## 工具错误

工具永远不能杀会话。`execute` 抛出的同步异常和被拒绝的 Promise，以及包住它的 `around` 抛错，都变成一条 `tool_result`：`isError` 为 true，正文是 `<工具名>: <message>`。这条结果写进下一条用户消息，模型下一轮看得到。连续多次也一样，默认看 5 次也不会追加 `outcome: "error"` 的 terminal。`runHealth.toolErrors` 只计数。

参数不是 JSON 对象的调用（流出的参数解析失败，或解析出数组、字符串这类非对象值）不执行工具，在 tool_pre 钩子与 guard 之后直接得到一条 `isError: true` 的 `tool_result`：`<工具名>: arguments are not a JSON object, so the tool did not run. ... Received: <原文>`，原文最多引 500 个字符。provider 回放这次调用时发的是占位 `{}`，模型只能从这条结果里看到自己发了什么。tool_pre 用 `updatedInput` 换成对象时照常执行。

会终止的是内核自己的失败和 provider 的失败：视图断言、没有适配器、整轮重试不重发的适配器 finish error、可重发的错误在整轮重试耗尽之后，以及工具管线以外的抛错。

构造传 `adapters`（provider 到适配器）。起始型号的 provider 不在里面时构造抛错。

## 整轮重试

流还没开始时的重试在 provider：`fetch` 抛错、读非 2xx 错误体时抛错、2xx 没有响应体、HTTP 429、HTTP 5xx，最多 2 次。响应体一开始，provider 就不再重试。

如果 finish 是可重发的错误，并且这一轮还没有把 assistant 消息写进 Log，core 重发同一条请求。可重发只看 `error.cause` 与它带的结构化字段，不看错误文案：

- `cause: "transport"`，传输失败，不管发生在流开始之前还是之后：`fetch` 抛错、读响应体抛错（SSE 体或非 2xx 的错误体）、2xx 没有响应体、SSE 体在收到结束信号前正常关闭。
- `cause: "event"` 且 `event.transient: true`，服务端在 200 的流里发来 error 事件，而事件的 `type` / `code` / `status` 不属于 provider 的不可重试类（请求非法、鉴权、内容过滤、上下文超长、额度耗尽、429 以外的 4xx）。分类规则见 `@sage/provider` README。

`cause: "http"` 不在这层重发，provider 已经在流开始前重试过 429 与 5xx。`event.transient: false` 的事件和 `cause: "other"` 也不重发。最多再发 2 次，先等 2 秒，再等 8 秒。构造参数 `sleep` 可注入，测试不真等。第 3 次仍失败，才把这次的 assistant 写入，并追加 terminal，outcome 为 `error`。

error 事件到达之前流里可能已经吐出半截文本或半截工具调用。它们只在这一次 `collect` 的结果里，重发时整条丢掉：assistant 没落 Log，工具也就没执行。

两层会叠加：流开始之前的传输失败先在 provider 用掉 3 次请求，core 再整轮重发 2 次，每次又是 3 次，最坏 9 次请求，退避合计约 25 秒。4xx 通常不重试；例外是 4xx 的错误体读到一半断开，连接断了、状态码不可信，按传输失败处理，两层都会重试。

每次重发之前追加 `turn_retry`，payload 是 `{ turn, attempt, reason }`，`reason` 是这次失败的 message。这条持久化，不进模型请求。被重发丢掉的半截文本与工具调用不进 Log。

## 运行时目录

`SAGE_DIR_NAME`（`.sage`）是 sage 在工作区和用户 home 下放自己文件的目录名：会话 Log、溢出的 bash 输出、子任务预设。定义在 core，是为了让 tools、plugins、apps 不互相 import 也能用同一个名字；各层怎么跳过它，写在各自的 README。

## 测试

无 key 是默认：测试进程会剥掉 `*_API_KEY`，并用录制的响应回放，不打真网。
`pnpm test:update` 重写期望产物，PR 里要逐行看这些 diff。
破坏前缀稳定会炸快照：head 变了，下一轮就不再是 `<last>` 加增量，而是整段重印。
