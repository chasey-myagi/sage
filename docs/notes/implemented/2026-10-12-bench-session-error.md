# bench 把会话错误判成错误类状态

## Problem

`bench run` 判定任务只看子进程是否超时、`git status` 是否为空、测试是否通过，不读会话怎么结束。provider 在第一轮就返回错误时（例如上游鉴权失败，助手消息以 `finish{kind:"error"}` 结束），会话 terminal `outcome` 是 `error`、token 为 0、工作区没改动，任务被记成 `empty-patch`，与「模型跑完了但没改东西」混在一起：`emptyPatchRate` 偏高，`resolvedRate` 偏低，对照实验无法从 `results.json` 剔除故障格（#162）。provider 错误的分类（状态码、`transport`）已在 Log 的助手消息 `details.finish.error` 里，却没有带到结果行。

## Decision

- 复制会话之后、判 diff 之前，读会话最后一条 `terminal` 的 `outcome`：`error` 记 `status: "error"`、`reason: "session-error"`；`aborted` 记 `reason: "session-aborted"`。两者都不看 diff、不应用 `testPatch`、不跑测试，`stoppedEarly` 为 null。`done` 与 `max_turns` 照旧往下判。超时仍先于这一步，记 `timeout`。会话读不到时跳过这一步，行为与之前相同。
- `session-error` 行带 `providerError`：最后一条助手消息以 `finish.kind === "error"` 结束时为 `{ status, transport }`，`status` 是 HTTP 状态码或 null，`transport` 照抄 `FinishError.transport`；会话因 provider 以外的原因出错（seam 断言、没有对应 adapter）时为 null。错误文本不写进 `results.json`。分类字段从 Log 一路带到结果行，不在 bench 里重新解释。
- outcome 为 `error` 但工作区有改动：仍记 `session-error`，不跑测试。会话没有正常结束，补丁是不是模型认定的完成态无从得知；若照常跑测试并允许 `resolved`，故障格会混进 `resolvedRate`，对照两边受上游稳定性影响的程度不同，差值就不再只反映上下文策略。判成错误类，故障格在两边都能按 `reason` 一眼识别、对称剔除。
- 报告口径：两种新原因计入现有的 `errorRate`，不新增比率字段。要看故障占比或剔除故障格，按 `results.json` 的 `reason` 过滤。
- 读 terminal outcome 的函数只留一份：`apps/sage/src/bench/session-end.ts` 的 `terminalOutcome`，早停检测改为引用它。

## Alternatives considered

- **outcome=error 但测试通过时仍记 `resolved`**：能多留下少数中途出错前已修好的样本，但故障格从此不可识别，违背「对照实验要能剔除故障格」的目标；且「测试通过」依赖隐藏测试，与 agent 是否完成无关。
- **新增 `providerErrorRate`**：报告多一个字段，`bench report` 的 `a` / `b` / `delta` 形状随之变化；`errorRate` 加 `reason` 过滤已足够定位，先不扩展报告形状。
- **按子进程退出码判断**：`sage -p` 对 `max_turns` 也退出 1，退出码区分不了「到了轮次上限」与「provider 故障」，也带不出分类字段。
- **只对第一轮出错的会话改判**：中途出错同样是故障，没有理由按轮次切分口径。

## Consequences

- `BenchErrorReason` 多两个成员；`BenchTaskResult` 多一个可选的 `providerError`，只在 `session-error` 行出现，其他行的形状不变。
- 被 SIGINT / SIGTERM 中断的子进程若写下了 `aborted` 的 terminal，记 `session-aborted`；bench 自己的超时仍记 `timeout`，因为超时判断在先。
- 历史 `results.json` 不重算：旧结果里的这类任务仍是 `empty-patch`，与新结果对比时要注意。
- `bench report` 读结果时不读 `providerError`，报告形状不变。

## Testing

`apps/sage/test/regressions/issue-162.test.ts` 通过 `runCli` 跑 `bench run`，注入的 `runSage` 写出指定 terminal outcome 与助手 `details.finish` 的会话：

- 第一轮 401、工作区无改动，记 `session-error` 并带 `{ status: 401, transport: false }`，不跑测试（修复前为 `empty-patch`，先红）。
- transport 失败带 `{ status: null, transport: true }`。
- terminal `error` 但最后一条助手消息不是错误 finish，`providerError` 为 null。
- terminal `error` 且工作区有改动，仍记 `session-error`，不跑测试。
- terminal `aborted` 记 `session-aborted`，不带 `providerError`。
- `max_turns` 结束的会话照旧：空 diff 为 `empty-patch`，有改动且测试通过为 `resolved`。

`pnpm check` 全绿。
