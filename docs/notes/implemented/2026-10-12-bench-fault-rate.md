# bench report 给出故障格占比，并可对称剔除故障格

## Problem

`bench run` 已把会话以 `error` / `aborted` 结束的任务记为 `session-error` / `session-aborted`，但 `bench report` 只给 `errorRate`：模型真失败（`tests-failed`）与上游、基础设施故障混在同一个数里，`resolvedRate` 的分母也包含故障格。两次运行各自在不同任务上碰到上游故障时，差值同时反映了策略差异和上游稳定性差异。做公平对照只能自己按 `results.json` 的 `reason` 过滤再重算，口径因人而异。

## Decision

- 「故障格」只在 `apps/sage/src/bench/fault.ts` 定义一处：`status` 为 `error`，且 `reason` 精确等于下列 `BenchErrorReason` 之一——`clone-failed`、`clone-unclean`、`setup-failed`、`sage-failed`（任务没交到 agent 手上）、`session-error`、`session-aborted`（会话因 provider / harness 错误或中断结束）、`diff-failed`（agent 的产出读不出来）。判定用 `switch` 穷举全部原因、以 `assertNever` 收尾，新增原因时编译器会要求归类；不读字符串子串。
- 不算故障：`tests-failed`；`test-patch-failed`，因为 agent 自己改了测试文件也会让隐藏的测试补丁打不上，归为故障会把 agent 造成的失败剔掉；`timeout`，慢或绕圈是策略效应，对照应该看见它。`results.json` 里不属于 `BenchErrorReason` 的字符串（手改或读坏的行）也不算。
- `BenchErrorReason` 改为由 `BENCH_ERROR_REASONS` 常量数组推出，读结果时用它把 `reason` 字符串收窄成枚举成员。
- `BenchMetrics` 加 `faultRate`（故障格数 / 任务数），`a` / `b` / `delta` 都带。`errorRate` 口径不变，仍含故障格。
- `bench report` 加 `--exclude-faults`，经共享参数表（`node:util` parseArgs）解析，目录前后都可写。开启时按任务 id 对称剔除：任一侧是故障格的任务两侧都去掉，再算全部比率。报告顶层新增 `excluded`，列出被剔除的 id，每个一次，先按 A 的顺序再补 B 独有的；不开启时为空数组，形状只有一种。

## Alternatives considered

- **只剔除各自那一侧的故障格**：两侧比较的任务集合不同，差值里仍混着「哪些任务碰上了故障」，正是要消除的偏差。
- **`excluded` 只在开启剔除时出现**：报告就有两种形状，下游脚本要分支处理；空数组已足以表达「没有剔除」。
- **把 `timeout` 算故障**：上游变慢确实会导致超时，但无法和 agent 绕圈区分；先按 agent 结果处理，需要时可新增结构化字段（例如区分 agent 进程超时与 `testCommand` 超时）再细分。
- **`errorRate` 改为不含故障格**：改变已有字段的含义，旧报告与新报告无法直接对照；新增 `faultRate`，两者相减即得 agent 自身的失败率。

## Consequences

- 报告形状变化：`a` / `b` / `delta` 多 `faultRate`，顶层多 `excluded`。已有用 `toEqual` 钉报告的测试同步加上 `faultRate: 0`。
- 开启剔除后两侧 `faultRate` 恒为 0，故障规模看 `excluded` 的长度或不开启时的 `faultRate`。
- 以 id 匹配两侧任务：只出现在一侧的任务照旧参与该侧计算。
- `sage --help` 与 bench usage 的 report 行改由参数表渲染。

## Testing

`apps/sage/test/regressions/issue-165.test.ts` 通过 `runCli` 跑 `bench report`，fixture 为虚构的 `results.json`：

- 不剔除时，`a` / `b` / `delta` 带 `faultRate`，`errorRate` 仍含故障格，`excluded` 为空（实现前先红）。
- 十种原因加一个未知字符串，故障格恰为七种。
- `status` 不是 `error` 的行即使带故障原因也不算。
- 两侧各在不同任务上有故障时，`--exclude-faults` 两侧只比较双方都没有故障的任务，`excluded` 列出被剔除的 id；`timeout`、`tests-failed` 保留。
- 旗标写在目录之后同样生效；两侧都故障的任务只列一次；未知旗标打印含 `[--exclude-faults]` 的 usage 并退出 1。

`pnpm check` 全绿。
