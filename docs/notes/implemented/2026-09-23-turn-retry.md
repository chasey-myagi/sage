# 流中途传输错误按整轮重试

## Problem

2026-09-23 入口生效 A/B 补跑里，darkreader-6747 第 31 轮和 dayjs-2377 第 11 轮，provider 的 finish 都是 `{kind:"error", error:{message:"terminated"}}`（undici 把流掐断）。会话 terminal `outcome: error`，前面几十轮作废。#72 只重试响应体开始之前的失败。流已经开始之后的断开故意不重试，怕同一段输出生成两遍。

## Decision

`AgentSession` 在一轮模型调用上重发同一条请求。条件是 finish 为 error、错误属于传输类、并且这一轮还没有把 assistant 写进 Log。传输类是消息里含有 `terminated`、`fetch failed`、`ECONNRESET`、`socket hang up`、`aborted by peer`，或者 finish error 上 `transport: true`。HTTP 4xx 不重试。

最多再发 2 次。第 1 次等 2 秒，第 2 次等 8 秒。构造参数 `sleep` 可注入。等待期间被 abort 则不再发，也不写 `turn_retry`。第 3 次仍失败，才把这次 assistant 写入并追加 terminal `outcome: error`。

每次真正重发之前追加 `turn_retry`，payload 是 `{ turn, attempt, reason }`。这条持久化，不进模型请求。`runHealth.turnRetries` 数路径上的条数。被丢掉的半截文本不进 Log。

流开始前的重试仍在 provider，不改那一层。

## Alternatives considered

不让 provider 在流已经开始之后自己重试。适配器这时已经把 delta 交给调用方。它若再 POST 一次，调用方手里同时有半截和第二次输出，对不上。provider 也不知道这一轮的 assistant 有没有落盘。落盘与否只有 core 知道：没落盘就重发同一请求，不会把半截写进下一轮视图，工具也还没跑，没有副作用要重放。

不把重试次数挂在 usage 上。#72 可以那么做，是因为成功或最终失败的那次调用仍有一条 assistant。流中途失败的那次 assistant 不落盘，没有条目可挂。所以单独用 `turn_retry`。

不在 core 里重跑已经执行过的工具。这次的失败发生在模型请求结束之前，工具管线还没开始。

## Consequences

两层分工固定：流前在 provider（1 秒 / 4 秒，次数在 usage.details），流中整轮在 core（2 秒 / 8 秒，`turn_retry`）。一次传输断开最多再等 10 秒。第 3 次仍失败，半截文本会随最后一次 assistant 进 Log，然后 terminal error。

`terminal.health` 多了 `turnRetries`。回放夹具补上 0。历史 jsonl 少这个字段仍能读，`runHealth` 按 `turn_retry` 重算。

`turn_retry` 不是模型可见输入。passthrough 只投影 message，下一次请求不会带上它。

## Testing

红灯时：第一次 `terminated` 的 `summary.outcome` 是 `error` 而不是 `done`；连续失败时 `health.turnRetries` 为 `undefined`；`transport: true` 同样直接 `error`。

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、84 个文件、407 个测试）。`packages/core/test/regressions/issue-85.test.ts` 覆盖：一次 `terminated` 后成功，Log 只有一条 assistant，前面有一条 `turn_retry`，terminal `done`，`turnRetries === 1`，退避 2000，第二次请求不含半截文本；连续 3 次 `terminated` 后 terminal error，`turnRetries === 2`，退避 2000 与 8000，前两次半截不进 Log；status 400 只请求一次。另覆盖 `fetch failed`、`ECONNRESET`、`socket hang up`、`aborted by peer` 和 `transport: true`。测试注入 sleep，不打真网。
