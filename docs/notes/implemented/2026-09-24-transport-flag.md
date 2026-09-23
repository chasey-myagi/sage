# 传输错误标记一路带到 finish

## Problem

#86 的整轮重试有两个判断来源：finish error 上的 `transport: true`，和 `TRANSPORT_MARKS` 文案匹配。前者在生产里从来不成立。`http.ts` 在 fetch 抛出时打了 `transport: true`，但 `retry.ts` 的 `fail()` 只复制 message/status/raw，`drive.ts` 的 `errorFinish` 也只收这三样，标记在两处都丢了。contract 的 finish.error 类型里也没有这个字段，`session.ts` 只能用 `"transport" in error` 加 `as` 硬读。

流中途断开根本不经过 `http.ts`。`parseSse` 读 body 时 undici 抛出 `terminated`，落进 `driveStream` 外层 catch，走 `failureFinish`，从来没有打标。所以整轮重试实际只靠文案。文案表之外的底层错误（`other side closed`、`getaddrinfo EAI_AGAIN` 等）不重试，会话直接 `outcome: error`。

## Decision

`transport?: boolean` 正式进 contract：`StreamChunk` 的 finish.error 抽成 `FinishError` 类型并导出。`stream.ts` 加一条不变量：`transport` 为 true，当且仅当失败发生在 HTTP 之下，即 fetch 抛出或读 body 抛出。HTTP 状态码、流内 error 事件、body 正常关闭但没有 stop，都不打标。

provider 一路带过去：`retry.ts` 的最终失败保留 `transport`；`drive.ts` 的 `errorFinish` 改成收一个对象，带上 `transport`；进入 `parseSse` 之后外层 catch 抓到的异常一律打标（此时只可能是读 body 抛出）。adapter 的 `onEvent` 抛出走内层 catch，不打标。

core 只读类型化字段。`turnTransportReason` 直接看 `finish.error.transport === true`，去掉 `as`，删掉 `TRANSPORT_MARKS`，也删掉 `isClientStatus`：带状态码的错误按不变量不会有 `transport`，这个分支已经不可能走到。

## Alternatives considered

保留 `TRANSPORT_MARKS` 作兜底。不选。两条传输失败路径（fetch 抛出、读 body 抛出）现在都在 provider 打标，文案表覆盖的每条消息都来自这两条路径。留着就是同一事实两个来源，而且文案表只认识几条英文串，换一个运行时或 undici 改个措辞就漏。分类属于 provider：只有它知道失败发生在哪一层。

把 body 正常关闭但没收到 stop（`stream ended before completion`）也算传输失败。不在这次做。#86 之前它就不在文案表里，不重试；改它是行为变化，不是修标记丢失。它和截断的代理超时确实很像，值得另开一件事评估。

在 session 里按 `status === undefined` 推断传输失败。不选。流内 error 事件和 adapter 解析异常同样没有状态码，会被误判。

## Consequences

整轮重试只认 provider 的标记。任何 `ProviderAdapter` 实现（包括测试替身）要让 core 重试，必须在 finish.error 上打 `transport: true`，只写 `terminated` 文案不再触发。

既有测试改动：`packages/core/test/regressions/issue-85.test.ts` 的 faux 输入里，`terminated`、`fetch failed`、`read ECONNRESET`、`socket hang up`、`aborted by peer` 这些 finish error 加上 `transport: true`。它们模拟的是真实 adapter 的输出，新契约下真实 adapter 对这些失败一定会打标。断言一条没改，400 用例保持不打标、不重试。

`session.ts` 文件头改写重试条件：看 `finish.error.transport`，不看文案；HTTP 状态错误不在这层重发，429/5xx 已在 provider 流前重试。`drive.ts`、`retry.ts`、`stream.ts` 文件头同步标记来源。Log 形状不变，`docs/log-manifest.md` 无需重新生成。

## Testing

新增 `packages/core/test/session/transport-flag.test.ts`，经过真实 `OpenAIAdapter`，注入 fetch，不打真网：

- 流中途读 body 抛出 `other side closed`（不在旧文案表里），Log 有一条 `turn_retry {turn:1, attempt:1, reason:"other side closed"}`，terminal `done`、`turnRetries === 1`，半截文本不进 Log。
- fetch 连续抛出 `getaddrinfo EAI_AGAIN api.example`，provider 流前重试 2 次后放弃，core 整轮重试 1 次后成功，Log 同样有一条 `turn_retry`，terminal `done`。
- faux 给出只有文案 `terminated`、没有标记的 finish error，不重试，terminal `error`。

红灯时前两条都是 `expected [] to deeply equal [ { turn: 1, attempt: 1, … } ]`：`turn_retry` 为空。

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、86 个文件、415 个测试）。
