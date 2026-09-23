# 响应身份进流契约

## Problem

Log 里 assistant 条目的 `details.responseId` 恒为空串，`details.responseModel` 是请求时的模型名，不是服务端实际回的模型。`session.ts` 调 `collect` 时把这两个值写死成 `""` 和 `this.model.model`。

真实值其实拿到了：两个 adapter 在解析 SSE 时把 id/model 写进挂在 iterable 上的旁路属性 `responseTrace`。这个属性不在 `ProviderAdapter` 契约里，core 看不见，只有 provider 的测试在读。为了挂这个属性，adapter 还手写了 `[Symbol.asyncIterator]` 包一层对象。

## Decision

响应身份作为流的一部分：`StreamChunk` 新增 `{ type: "response", id, model }`。`stream.ts` 加不变量：至多一个 response chunk，出现在任何 `block_start` 之前；流前就失败的调用没有它。

- Anthropic：`message_start` 推出 response chunk（`message.id`、`message.model`）。
- OpenAI 兼容：第一个解析成功的 SSE 对象推出 response chunk（`id`、`model`），之后不再推。
- `collect(chunks, { api })`：meta 只剩 `api`，id/model 从 response chunk 取；没有 response chunk 时两者都是 `""`。`AssistantMessage` 的这两个字段仍是 `string`，Log 形状不变。
- 删掉 `responseTrace` 与手写的 `[Symbol.asyncIterator]` 对象，`stream()` 直接返回 `driveStream` 的生成器。
- `session.ts` 与 `plugins/src/scorer/model.ts` 都改为 `collect(stream, { api })`。

## Alternatives considered

放进 finish 或 usage chunk。不选。身份在流的最开始就知道，和结束原因、token 计数不是同一个事实；放进 finish 会让中途断开的调用丢掉已经收到的 id，而这正是排查断流时最想要的。

保留旁路属性，把它升格进 `ProviderAdapter` 契约（例如 `stream()` 返回 `{ chunks, trace }`）。不选。那是第二条通道，调用方必须在流读完之后再去读另一个对象，时序靠约定；放进流里，collect 一处消化，faux 与回放脚本也能直接写出来。

没有 response chunk 时让字段缺席（改成可选），而不是空串。不选。`request.test.ts` 断言 `responseModel`/`responseId` 是 string，历史 Log 也一直是 string；空串表示「服务端没给身份」，请求模型已经在 `request_header` 里，不再在这里重复一份。

没有 response chunk 时回退到请求模型名。不选。这正是原来的 bug：把请求值冒充响应值，同一事实两个来源。

## Consequences

`stream()` 返回的是单次可迭代的生成器，不再能对同一个返回值迭代两次。仓内没有这样用的地方，session 的整轮重试每次都重新调用 `stream()`。

faux 与回放脚本想让 Log 里有身份，就要在脚本里写 response chunk；不写则得到空串。

既有测试改动：

- `packages/provider/test/contract-suite.ts`：`expectStreamInvariants` 增加 response chunk 的不变量检查。`runContractSuite` 的 fixtures 多一个 `response` 期望值。scenario 1 期望 `[response, ...SHARED_HAPPY]`。scenario 5 原来在第 2 个 chunk 后 abort 并断言前两个是 `SHARED_HAPPY[0..1]`，现在第 1 个是 response chunk，改为第 3 个 chunk 后 abort，断言 `chunks[0]` 是 response、`chunks[1..2]` 仍是 `SHARED_HAPPY[0..1]`，其余断言不变。
- `packages/provider/test/adapters.test.ts`：两处 `runContractSuite` 传入各自 fixture 的 response；cross-adapter 用例先分别断言两边第一个 chunk 是各自的 response（`msg_01`/`claude-sonnet-4-5`、`chatcmpl-abc`/`gpt-5`），再比较其余 chunk 逐个相等；deepseek 用例的 `collect` 调用改签名。
- `packages/provider/test/collect.test.ts`：`collect` 改签名；第一个用例把 id/model 从 meta 移到流里的 response chunk，断言不变；两个 adapter 用例原来断言 `stream.responseTrace`，现在断言 collect 出来的 `message.responseId`/`responseModel` 等于同样的值；新增一例：没有 response chunk 时两者为空串。
- 回放夹具 `packages/core/test/replay/{text-only,tool-roundtrip}/responses.json` 每轮开头加 response chunk（model `claude-test`，id `msg_replay_*`），`session.jsonl` 对应 assistant 的 `responseId` 由 `""` 改成同一 id。`responseModel` 保持 `claude-test`，回放比较逻辑不变。
- `packages/core/test/support/faux.ts` 的 `FauxChunk` 类型补上 response 分支。

文件头同步：`stream.ts`（不变量）、`collect.ts`、`adapter.ts`（`AssistantMessage` 字段含义）、两个 adapter、`session.ts`、`testing/faux.ts`。Log entry 与 kind 不变，`docs/log-manifest.md` 无需重新生成。

## Testing

新增 `packages/core/test/session/response-identity.test.ts`，经过真实 `AnthropicAdapter`/`OpenAIAdapter`，注入 fetch，不打真网。wire 上的模型名故意和请求模型不同（`claude-sonnet-4-5-20250929` 对 `claude-sonnet-4-5`，`gpt-5-2026-08-07` 对 `gpt-5`），断言 Log 中 assistant 条目的 `details.responseId`/`responseModel` 等于 wire 值。红灯时两例都失败：`responseId` 为 `""`，`responseModel` 为请求模型名。

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、87 个文件、418 个测试）。
