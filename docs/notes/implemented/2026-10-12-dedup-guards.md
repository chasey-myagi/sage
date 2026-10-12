# Agent Note

## Problem

同一个小工具在各包里各写一份：

- 有限数守卫 6 份具名定义：core `health.ts` 的 `finite`、`log/kinds.ts` 的 `isFiniteNumber`、plugins `guards.ts` 的 `finite`、`score-common.ts` 的 `finiteNumber`、provider `record.ts` 的 `num`、apps `guards.ts` 的 `isFiniteNumber`，另有十几处内联的 `typeof x === "number" && Number.isFinite(x)`。
- 从 Log 读回 usage 三份：`health.ts` 的 `readTokenCounts`、`score-common.ts` 的 `finiteUsage`、`replay/offline.ts` 的 `inputTotal`。
- `core/log/types.ts` 有一份 `ModelRef` 结构替身；apps 的 `LadderRung` 与 `ModelRef` 同形；`provider/model` 字符串解析在 `options.ts` 与 `subtask/presets.ts` 各一份；`isModelRef` 在 `kinds.ts` 与 `task-input.ts` 各一份。
- tools 的 `find` 与 grep 进程内搜索各自手写目录遍历。
- 打分器截止时间控制器在 `scorer/jev.ts` 与 `scorer/model.ts` 各一份。`jev.ts` 重试前的 sleep 没传 `signal`，调用方取消后还要把退避睡完（最多 2 秒），醒来后再发一次已被取消的请求。

## Decision

- `@sage/provider/contract` 的 `record.ts` 加 `isFiniteNumber` 与 `finiteOr(value, fallback)`，`usage.ts` 加 `readTokenCounts(value)`（非对象返回 undefined，缺失或非有限的计数读作 0）。core 照 `isRecord` 的做法转出这三个。各包的具名副本删掉，内联判断改用 `isFiniteNumber`；类型已经保证是 `number` 的地方（`decide.ts`、`policy.ts`、`presets.ts` 的 `Number(value)`、`resolveMinScoreTokens`）不动。
- `health.ts`、`score-entry.ts`、`score-shadow.ts`、`offline.ts` 都改用 `readTokenCounts`。`calibrate.ts` 的 `inputTotal` 是严格版：任一输入计数缺失就返回 undefined，不当 0 处理，否则会拉偏校准比例。保留它，并在函数上写一句说明差异。
- 新增 `contract/model-ref.ts`，放 `isModelRef` 与 `parseModelRef`。`log/types.ts` 的替身删掉，`kinds.ts` 直接用 contract 的 `ModelRef`；`LadderRung` 删掉，换成 `ModelRef`；两份解析与两份守卫各合成一份。
- tools 新增 `walk.ts` 的 `walkWorkspaceFiles(root)`，返回未排序的绝对路径。`find` 与 grep 进程内搜索各自过滤、排序。逐项对齐：不跟随符号链接；跳过 `SKIPPED_DIR_NAMES` 里的名字（文件也跳过）；根是文件时列出它自己；根或目录读不了就跳过；结果由调用方排序。
- plugins 新增 `scorer/deadline.ts` 的 `startDeadline`，用 `AbortSignal.any([AbortSignal.timeout(ms), callerSignal])` 合成信号，在合成信号的 abort 事件里判定先到的是哪一路。`AbortSignal.timeout` 的计时器清不掉，所以不再需要 `dispose`。
- `jev.ts` 重试的 sleep 传入调用方的 `signal`；醒来时如果已经取消，直接返回 `aborted`，不再发请求。

## Alternatives considered

- 只导出 `isFiniteNumber`，让调用方自己写 `isFiniteNumber(x) ? x : 0`。大多数调用点是「取值，不是有限数就用默认值」，每处一个三元表达式就是把同一个事实又展开了十几份。
- `readTokenCounts` 放在 core。provider 的类型面本来就拥有 `Usage`；放在 contract，core、plugins、apps 都够得到，也不新增 core 的导出面以外的东西。
- 截止时间控制器保留手写的 `AbortController` + `setTimeout`。`AbortSignal.any` / `AbortSignal.timeout` 是 Node 内置的，能删掉手写的监听器注册与清理。

## Consequences

- 判断「是不是有限数」「从 Log 读 usage」只有 contract 一个定义，新代码直接从 `@sage/core` 或 contract 取。
- `AbortSignal.timeout` 的计时器在调用结束后仍然挂着，直到超时（默认 10 秒），但它是 unref 的，不会拖住进程退出。
- `jev` 在退避期间被取消时，fallback 是 `aborted`，不会多睡、也不会多发一次请求。

## Testing

- `pnpm check` 全绿（95 个测试文件、504 个用例）。快照、回放与 `issue-*` 回归用例的断言没有改动；`issue-50.test.ts` 里一个本地 helper 原来 import 了被删的 `plugins/src/guards.ts`，改为用 `readTokenCounts` 与 `inputTokens` 实现同样的求和。
- 红灯先行：`hands the caller's signal to the retry wait and does not fetch again once it aborts` 在改动前失败（sleep 收到的 signal 是 undefined），改动后通过。
- 目录遍历的等价性：`packages/tools/test/walk.test.ts` 的两条用例（跳过 `node_modules`、`.git`、文件与目录符号链接、结果排序；根是文件、根不存在）先对改动前的 `find.ts` / `grep.ts` 跑过，结果相同，再对改动后的代码跑过。
- 新增与改动的测试文件用一份临时 tsconfig 做过类型检查，没有新增类型错误。
