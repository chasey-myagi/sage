# 未完成的流按传输失败重试

## Problem

SSE body 正常关闭、却没有收到结束信号（OpenAI 的 `[DONE]`、Anthropic 的 `message_stop`），provider 以 `finish{kind:"error", error:{message:"stream ended before completion"}}` 结束，不带 `transport`。core 的整轮重试只认这个标记，于是不重试，会话直接 `outcome: error`，前面几十轮作废。

这和代理超时截断连接是同一类失败：连接在响应完成之前结束了。区别只在底层是抛错（undici `terminated`）还是像正常 EOF 一样关闭，取决于代理怎么断、运行时怎么报。上一版契约把前者算传输失败、后者不算，分类取决于偶然的表现形式。

2xx 响应没有 body 也一样：`http.ts` 返回同一条消息，不带标记，provider 不做流前重试，core 也不整轮重发。

## Decision

「响应在完成之前结束」归为传输失败，打 `transport: true`。产生 `INCOMPLETE_STREAM` 的地方逐一确认：

- `http.ts` 的 `!response.body`：2xx 没有 body，打标。它发生在流开始之前，`retry.ts` 按传输失败在 provider 层重试（1 秒、4 秒），耗尽后 finish 带标记，core 再整轮重发。
- `drive.ts` 的 `readBody` 读到 EOF 仍没有 stop：打标，core 整轮重发。
- `http.ts` 的 `messageOf`：抛出的值没有消息时的回退文案。经 fetch 抛错、读 body 抛错的路径本来就打标；经 adapter `onEvent` 抛错的路径是解析缺陷，不打标，不变。
- `contract/collect.ts`：流里没有 finish chunk 时的默认值。那是 adapter 违反契约，不是传输失败，不打标，不变。

`contract/stream.ts` 的不变量改写：`transport` 为 true 当且仅当失败发生在 HTTP 之下，包括 fetch 抛错、读 body 抛错、响应在完成前结束（2xx 没有 body，或 SSE body 没收到 stop 就关闭）。读完错误体的 HTTP 状态和流内 error 事件仍不打标。当时两个 adapter 还不解码流内 error 事件，服务端报错后关连接会落进「未收到 stop」而被打标；后来改为解码成不打标的 finish error，见 [流内 error 事件](./2026-09-24-stream-error-event.md)。

整轮重发是安全的，论证同 [整轮重试](./2026-09-23-turn-retry.md)：finish 为 error 时这一轮的 assistant 还没落 Log，工具还没执行，重发同一请求不会重放副作用，半截文本也不进下一轮视图。

## Alternatives considered

只在 SSE 读到 EOF 时打标，`!response.body` 不动。不选。两者都是「连接在响应完成前结束」，同一消息两种分类，正是这次要消掉的偶然性。

按消息文案 `stream ended before completion` 在 core 里判断。不选。状态不编码进字符串；分类属于 provider，只有它知道失败发生在哪一层。

在 OpenAI 侧把「已收到 finish_reason、只缺 `[DONE]`」当作正常完成。不选。那是另一件事：放宽完成的判定，而不是给失败分类；而且 usage 往往在 finish_reason 之后的帧里，提前收尾会丢 usage。

## Consequences

body 未完成就关闭的调用，core 最多再整轮重发 2 次（2 秒、8 秒），每次写 `turn_retry`，reason 是 `stream ended before completion`。第 3 次仍失败，半截文本随最后一次 assistant 进 Log，terminal error，与其他传输失败一致。

2xx 没有 body 的调用先在 provider 重试，次数记在 usage 上。

既有测试不改断言：`packages/provider/test/contract-suite.ts` 与 `packages/provider/test/regressions/issue-72.test.ts` 对截断流用 `toMatchObject` 断言 message，多出的 `transport` 不影响。Log 形状不变，`docs/log-manifest.md` 无需重新生成。

文件头与文档同步：`contract/stream.ts`、`drive.ts`、`http.ts`、`retry.ts`、`core/src/turn-retry.ts`、provider 与 core 的 README、`docs/defensive-patterns.md` 的重试分层一节，以及 [传输标记](./2026-09-24-transport-flag.md) 里「不在这次做」那段改为指向本文。core README 里残留的文案表描述一并删掉。

## Testing

新增 `packages/core/test/session/incomplete-stream.test.ts`，经过真实 adapter，注入 fetch，不打真网：

- OpenAI：第一次 body 吐出 `half-` 后正常关闭（没有 `[DONE]`），第二次正常。Log 有一条 `turn_retry {turn:1, attempt:1, reason:"stream ended before completion"}`，terminal `done`、`turnRetries === 1`，半截文本不进 Log。
- Anthropic：body 在 `message_delta` 之后、`message_stop` 之前关闭，断言同上。
- 2xx 没有 body，第二次正常：provider 流前重试，没有 `turn_retry`，terminal `done`。

红灯时前两条是 `expected [] to deeply equal [ { turn: 1, attempt: 1, … } ]`，第三条 terminal 为 `error`。
