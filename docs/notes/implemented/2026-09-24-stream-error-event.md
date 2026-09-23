# 流内 error 事件解码为非传输错误

## Problem

服务端可以在 200 响应的 SSE 流里明确报错，然后关连接。Anthropic 发 `event: error`，data 形如 `{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}`；OpenAI 兼容接口发 `data: {"error":{"message":…,"type":…,"code":…}}`。

两个 adapter 都不解码它。Anthropic 的 `interpret` 对 `type: "error"` 走 `default: return`，OpenAI 的帧没有 `choices` 也被跳过。连接随后关闭，`drive.ts` 看到 body 结束却没有 stop，产出 `INCOMPLETE_STREAM` 并打 `transport: true`。core 据此整轮重发两次，服务端的原始错误文本丢失，Log 里只剩 `stream ended before completion`。

`contract/stream.ts` 的不变量说 `transport` 在「当且仅当」失败发生在 HTTP 之下时为 true，并声称流内 error 事件不打标。实现做不到：流内 error 事件恰好被打标。

## Decision

`Driver` 增加 `fail(raw)`，与 `complete` 共用「先到先算」：一个事件调用它之后，`readBody` 交出这个事件已经排队的块，然后结束，不再解码后面的帧。finish 为 `error`，`message` 取 `raw.error.message`（没有则取顶层 `message`，再没有则是固定文案），`raw` 是整个事件对象，不带 `transport`，不带 `status`。取 message 的逻辑与错误体共用 `http.ts` 的 `messageFromPayload`。

Anthropic 在 `type: "error"` 上调 `fail`；OpenAI 在帧带 `error` 对象时调 `fail`，并且放在宣告 response 之前，error 帧不算「命名响应的帧」。

`contract/stream.ts` 的不变量补上这类失败的形状：服务端文本、`raw` 为事件对象、之后没有块。provider README 加一段说明。[未完成的流](./2026-09-24-incomplete-stream.md) 里「流内 error 事件仍不打标」一句补注当时实现与之不符、现已修正。

## Alternatives considered

把流内 error 事件也打成 `transport: true`，让 core 重发。不选。服务端已经给出结论（过载、内容被拦、上下文超长），这和连接意外中断是两回事；有的错误重发必然再错，白白多 2 次请求和 10 秒等待。过载类错误是否值得重发是另一个决定，要按服务端的错误类型判断，而不是借用传输标记。

在 adapter 里直接抛异常，走 `onEvent` 抛错的路径。不选。那条路径的 message 是异常文案，`raw` 丢失，而且把一个正常的失败结果当成解析缺陷处理，违反「失败是正常结果时返回值」。

让 adapter 自己 push 一个 finish chunk。不选。finish 与 usage 的顺序由 `drive.ts` 统一保证，adapter 只记录结局。

## Consequences

流内 error 事件结束的调用不再被 core 整轮重发。这次 assistant（带着事件前已经交出的半截文本）进 Log，`details.finish.error.message` 是服务端文本，terminal `outcome: error`。

`Driver` 多一个方法，将来新增 adapter 遇到协议里的错误事件时调用 `fail`，而不是忽略它。

Log 形状不变，`docs/log-manifest.md` 无需重新生成。

## Testing

`packages/provider/test/contract-suite.ts` 的失败表加一行「the stream carries an error event」，两个 adapter 共享，夹具为 `fixtures/anthropic-error-event.sse` 与 `fixtures/openai-error-event.sse`：先吐 `Hi`，再是 error 事件，再是一段 ` after`。断言 finish 为 error、message 为 `Overloaded`、没有 `transport` 与 `status`。另加一例断言 text delta 只有 `Hi`、`raw.error.message` 为 `Overloaded`。

`packages/core/test/session/stream-error-event.test.ts` 经真实 adapter 注入 fetch：只请求 1 次，没有 `turn_retry`，terminal `outcome: error`、`turnRetries: 0`，assistant 的 `finish.error.message` 为 `Overloaded`、没有 `transport`。

修复前 6 条全红：provider 两例 `expected [ 'Hi', ' after' ] to deeply equal [ 'Hi' ]` 与 `expected true to be false`（transport 被打标），core 两例 `turn_retry` 非空。
