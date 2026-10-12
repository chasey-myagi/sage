# Agent Note: FinishError 以 `cause` 判别失败来源

## Problem

`FinishError` 的三种失败来源是并列的可选字段：`transport?: boolean`、`status?: number`、`event?: StreamErrorEvent`。三者互斥只靠 `stream.ts` 文件头的不变量和契约测试维持，类型上 `{ transport: true, status: 502 }`、`{ event, status }` 都合法，什么都不带的 `{ message }` 也合法。消费方（core 的整轮重试、bench 的 `providerError`）各自用 `transport === true || event?.transient === true` 这类组合判断，新增一种来源时编译器不会指出哪里漏了。`assertNever` 只在 `apps/sage` 有一份，core 用不到。

另外，core 测试里「按次序回 SSE 响应并计数」的 fetch 替身在 `transport-flag`、`incomplete-stream`、`stream-error-event` 三个文件里各写了一份，`@sage/core/testing` 已有 `scriptedSseFetch`。

## Decision

- **`FinishError` 是判别联合**：`{ message; raw? } & ({ cause: "transport" } | { cause: "http"; status } | { cause: "event"; event } | { cause: "other" })`。成员按现有失败路径定：
  - `transport`：`fetch` 抛错、读响应体抛错（SSE 体或非 2xx 错误体）、2xx 没有 body、SSE 体没收到结束信号就关闭。
  - `http`：非 2xx 且错误体已读完。`status` 必有。
  - `event`：200 流内的 error 事件。`event` 必有。
  - `other`：adapter 的 `onEvent` 抛错，以及 `collect` 在 chunk 流没有 finish 时补的默认值。两者以前都是「什么标记都没有」，现在有了名字。
- **`PostFail` / `OpenedCall` 跟着收紧**：`http.ts` 导出 `PostError = Extract<FinishError, { cause: "transport" | "http" }>`，失败结果把它放在 `error` 字段里原样带到 `drive.ts`，`errorFinish` 直接收一个 `FinishError`，中间不再逐字段拷贝。被 abort 的 `PostFail` 不再带 `message`，没有调用方读它。`retry.ts` 的可重试判断对 `cause` 做 `switch`。
- **消费方对 `cause` 做 `switch`，以 `assertNever` 结尾**：core 的 `turnRetryReason` 重发 `transport` 和 `transient` 的 `event`，不重发 `http` 与 `other`，行为与之前相同。bench 的 `providerError` 先用 `isFinishErrorCause` 收窄 Log 里的 `cause`，再按 cause 产出同构的 `BenchProviderError`。
- **`assertNever` 下沉到 `@sage/provider/contract`**，经 `@sage/core` 转出；删掉 `apps/sage/src/guards.ts`，三处调用改从 core 引用。
- **`isFinishErrorCause` 与类型放在一起**：cause 集合用 `Record<FinishError["cause"], true>` 定义，少一个或多一个 key 都编译不过。
- **类型约束由 `tsc -b` 检查**：各包 tsconfig 只包含 `src/`，vitest 也不做类型检查。`packages/provider/test/types/` 是一个独立的 TS 工程（`emitDeclarationOnly`，产物进被忽略的 `dist/`），挂在根 `tsconfig.json` 的 references 里。里面的 `@ts-expect-error` 钉住非法组合：transport 带 status、http 缺 status、http 带 event、event 缺 event、other 带 status、没有 cause、旧的 `transport` 标记，以及未收窄时读 `status` / `event`。约束一旦放宽，指令变成未使用，`pnpm check:types` 和 `pnpm build` 都会报错。
- **`scriptedSseFetch` 接受脚本步骤**：字符串仍是 200 SSE body；另有 `{ kind: "no-body" }`、`{ kind: "throw", message }`、`{ kind: "cut", body, message, status? }`。三个测试文件删掉自建替身改用它。
- **bench 的 `results.json` 改形状**：`providerError` 从 `{ status, transport, event }` 变成上述四种之一。Log 里 `cause` 缺失、不认识，或 `http` 没有数字 `status`、`event` 没有可读的 `event`，记为 `{ cause: "other" }`。预发布不留兼容层（G4），旧结果文件里的 `providerError` 不再按旧字段解读；`bench report` 本来就不读它。

## Alternatives considered

- **`raw` 只放在 `http` 与 `event` 两个成员上**：更精确，但 `raw` 目前没有按来源分支的读者，放在公共部分与 issue 的设想一致，改动面更小。以后有读者按来源取 `raw` 时再收紧。
- **`collect` 的默认 finish 记为 `transport`**：消息同样是 `stream ended before completion`，但它表示生产方违反了流约定（没有 finish chunk），不是连接问题；以前它也不触发整轮重试。记为 `other`，行为不变。
- **把所有测试目录纳入类型检查**：core、plugins、apps 的测试目录现在有几十处类型错误，修它们是另一件事。只为这条约束开一个最小的类型测试工程。
- **改用 vitest 的 `typecheck` 模式与 `*.test-d.ts`**：要在 vitest 里再起一个 tsc，并且依赖它对 TypeScript 7 的支持；挂进现有 `tsc -b` 图更直接，也跟着 `pnpm check` 走。
- **bench 遇到无法识别的 cause 时 `providerError` 记为 null**：null 已经表示「会话因 provider 以外的原因出错」，混用会让两种情况分不开。

## Consequences

- 构造 `FinishError` 必须写 `cause`；faux 脚本里的错误 finish 也要带，否则 core 的 `switch` 落到 `assertNever` 抛错。测试里的几处 faux finish 因此补了 `cause`。
- 新增一种 cause 时，`isFinishErrorCause` 的表、core 的整轮重试、bench 的归类三处编译报错，必须逐一决定怎么处理。
- `results.json` 的 `providerError` 形状变化，读它的外部脚本要按 `cause` 改写。
- Log 里 assistant 条目的 `details.finish.error` 多了 `cause`，不再有 `transport`。它不是 entry 种类的变化，`docs/log-manifest.md` 不用重生。

## Testing

- `packages/provider/test/types/finish-error.ts`：先于实现写成，当时 `tsc -b` 在这个文件报错（`cause` 不存在、`@ts-expect-error` 未使用），改后通过。另做了两次变异：给 `transport` 成员加 `status?` 后报未使用的 `@ts-expect-error`；联合里加一个 `probe` 成员后 `turn-retry.ts` 与 `session-end.ts` 的 `assertNever` 处报错。
- `packages/provider/test/contract-suite.ts`：每种失败的 finish 断言 `cause`，`http` 才有 `status`，`event` 才有 `event`。原先断言「标不标 transport」的用例改为断言 cause，用例数不变。
- `packages/provider/test/drive.test.ts`：adapter 的解码函数抛错时，finish 为 `cause: "other"`，之前排进队列的块照常产出。旧实现的这个 finish 不带 `cause`，`toEqual` 不成立。
- `apps/sage/test/regressions/issue-162.test.ts`：`providerError` 按新形状断言；新增一例，cause 缺失、未知或专属字段损坏时记为 `other`。
- `packages/core/test/session/{transport-flag,incomplete-stream,stream-error-event}.test.ts` 改用 `scriptedSseFetch`。断言不变，另加了调用次数断言，补上自建替身「多调用一次就抛错」那层约束。`stream-error-event` 里的 `not.toHaveProperty("error.transport")` 由 `cause: "event"` 取代。
- `pnpm check` 全绿。
