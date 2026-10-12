# Agent Note: 流内 error 事件按结构化分类整轮重发

## Problem

服务端可以在 200 响应的 SSE 流里发 error 事件（Anthropic 的 `event: error`，OpenAI 兼容接口带 `error` 对象的帧）。经 OpenAI 兼容中转站时，上游限流、上游流卡住这类瞬时故障就是这样到达的。两层重试都不接：provider 只在响应体开始之前重试，core 的整轮重试只认 `finish.error.transport === true`，而 [流内 error 事件](./2026-09-24-stream-error-event.md) 刻意不打 `transport`。会话直接以 `outcome: error` 结束，`turnRetries: 0`。

那份 Note 留了一句：过载类错误是否值得重发是另一个决定，要按服务端的错误类型判断，而不是借用传输标记。finish 上也没有任何字段能让 core 或 bench 区分「流内 error 事件」与「不带状态的其他失败」，`providerError` 只能报 `{ status: null, transport: false }`。

## Decision

- **新分类挂在 `FinishError.event`**，类型 `StreamErrorEvent = { transient: boolean; type?; code?; status? }`，只在调用结束于流内 error 事件时出现。它与 `transport`、`status` 并列：`transport` 是 HTTP 以下的失败，`status` 是读完错误体的 HTTP 状态，`event` 是 200 流里服务端的报错。三者互斥，`event` 存在时 `transport` 与 `status` 都不设（HTTP 状态是 200，不能把事件里的 `status` 冒充 HTTP 状态）。`type` / `code` / `status` 从事件的 `error` 对象按类型读出，类型不符就不带；`code` 保留字符串或数字原值。
- **`transient` 由 provider 判断**，core 只读布尔值。错误类型的词表是各家协议的事，core 不认识 adapter，判断只能放在 provider。`drive.ts` 的 `classifyEvent` 统一读字段；每个 adapter 通过 `SseCall.permanentErrors` 交一张不可重试表（两个 adapter 都有，对称）。命中表的 `type` 或字符串 `code`，或 `status` 是 429 以外的 4xx，`transient` 为 false；其余为 true。状态码的判断与 `retry.ts` 的 HTTP 重试共用 `isRetryableStatus`，同一个事实一个定义。
- **默认瞬时，按黑名单排除。**没有任何结构化字段的事件（中转站常见）算瞬时。只认结构化字段，从不读 message 文本。数字 `code` 只照抄、不当状态码：有的接口把 HTTP 码放在 `code`，有的放业务码，含义不一致。
- **core 的 `turnRetryReason`** 在 `transport === true` 或 `event.transient === true` 时给出 reason。其余约束不变：本轮尚未追加 assistant 消息才重发，最多 `TURN_RETRY_LIMIT` 次，每次先写 `turn_retry`。
- **部分内容**：error 事件之前流出的半截文本或半截工具调用只存在于这一次 `collect` 的结果里。`collectWithTurnRetry` 决定重发时直接丢掉它，assistant 没落 Log，工具没执行，`tool_result` 不会出现。最后一次仍失败时，这次 assistant 照旧带着半截内容和 finish 落 Log，与传输错误耗尽时一致。
- **bench**：`providerError` 增加 `event: { transient, type, code, status } | null`，缺的字段为 null。事件的 message 仍不复制。

## Alternatives considered

- **把流内 error 事件打成 `transport: true`**：已在前一份 Note 否掉。它让「连接断了」和「服务端说话了」混成一个标记，bench 也分不出来。
- **在 core 维护不可重试的 `type` / `code` 表**：core 就得认识 Anthropic 与 OpenAI 的错误词表，违反「core 不认识 adapter」的方向；换一个 adapter 还得改 core。不选。
- **白名单（只重发 `overloaded_error`、`rate_limit_error`、`server_error` 等已知瞬时类）**：中转站的错误帧多半没有 `type` / `code`，或用自造的值，白名单会让本次要修的场景照旧不重试。不选。代价是未知的永久错误会被重发两次：多 2 次请求、10 秒等待，结局不变（仍是 `outcome: error`，带同一条 message）。这个代价可以接受，并且只要服务端给出可识别的 `type` / `code`，就能加进表里排除。
- **把事件的 `status` 写进 `FinishError.status`**：`status` 的含义是 HTTP 状态，下游（bench、`retry.ts` 的规则）按 HTTP 语义读它。放进 `event.status`，一个词一个意思。
- **`event` 用判别联合替换 `transport` / `status`**（如 `cause: "transport" | "http" | "event"`）：形状更整齐，但要改动所有生产与测试里的 `transport` 读写，范围超出这件事。本次只新增字段；三者互斥在 `contract/stream.ts` 的不变量里写明。

## Consequences

- 瞬时的流内 error 事件现在会被整轮重发，最多多 2 次请求、10 秒退避。provider 层不变：流已开始，不在本包重试。
- 新 adapter 必须提供 `permanentErrors`（`SseCall` 的必填字段），编译器会提示。
- `packages/core/test/session/stream-error-event.test.ts` 原本钉的是「`overloaded_error` 不整轮重发」，这正是被本决定淘汰的行为。改成钉「不可重试类（`invalid_request_error`）不整轮重发，且 finish 带 `event: { transient: false, type }`」；瞬时事件的重发由回归测试覆盖。
- `turn_retry` 的 entry doc 措辞变了，`docs/log-manifest.md` 重新生成；payload 形状不变。

## Testing

- `packages/provider/test/regressions/issue-169.test.ts`：两个 adapter，一张事件分类表（只有 message、429 / 503 / 400 的 `status`、数字 `code`、未知 `type`、各自的永久类与瞬时类），断言 `finish.error.event` 的精确形状，且不带 `transport` 与 `status`。修复前全部红（`event` 为 undefined）。
- `packages/provider/test/contract-suite.ts`：error 事件夹具的 finish 带 `event`，两个 adapter 各给期望值（`overloaded_error` / `server_error`，均瞬时）。
- `packages/core/test/regressions/issue-169.test.ts`：经真实 adapter 注入 fetch。瞬时事件在半截文本后到达 → 请求 2 次、一条 `turn_retry`、`outcome: done`、Log 里没有半截文本；在半截工具调用后到达 → 同上且没有 `tool_result`；一直失败 → 3 次请求、2 次 `turn_retry`、`outcome: error`，最后的 finish 带 `event: { transient: true }`；永久类 → 1 次请求、没有 `turn_retry`。修复前瞬时三例各两个 adapter 共 6 例红。
- `apps/sage/test/regressions/issue-162.test.ts`：`providerError` 带 `event`；流内事件的结果行带 `{ transient, type, code, status }`，缺的字段为 null，结果里不含 message 文本。修复前红。
- `pnpm check` 全绿。
