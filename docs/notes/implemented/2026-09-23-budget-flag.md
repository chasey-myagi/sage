# --budget

## Problem

真机对照（deepseek-flash，ts-mini 10 条）里，compiler 组每轮输入 4k–25k，从没碰到默认预算 40000。10 条全部 `omitted=0`、`epochs=1`。`createCodingSession` 已经能收 `budget`，但 `sage -p` 和 `bench run` 都没有把这个参数露出来，对照没有测到编译器。

## Decision

`sage [-p] <task> --budget <tokens>` 把正整数透传给 `createCodingSession`。不是正整数时退出 2，stderr 一行点名这个值。`--passthrough` 不把该值交给编译器，并在 stderr 写一句说明。

`sage bench run --budget <tokens>` 把同一对参数放进每个任务的 `sage -p` argv，passthrough 也放。`results.json` 顶层 `budget`：compiler 且写了旗标时是这个整数；没写旗标，或策略是 passthrough，为 `null`。`bench report` 的 `a` 和 `b` 各带同一字段，`delta` 不含。

## Alternatives considered

为什么没写旗标时不回填 40000。40000 是编译器内部默认，不是这次对照压下去的数。结果文件写成 `null`，才能看出这组没有压预算。passthrough 即使 argv 里带了 `--budget` 也记 `null`，因为 sage 会忽略，清单上没有 budget。

为什么 passthrough 的 argv 仍然带上旗标。忽略发生在 sage：stderr 留一句，而不是 bench 把旗标丢掉。这样两组命令行的差别只剩 `--passthrough`。

为什么不把正整数检查抽成新模块。`sage` 和 `bench run` 各留一份相同的检查和同一句错误。八行不值得再开一个文件。

## Consequences

默认预算仍是 40000。短任务不写 `--budget` 不会触发淘汰。对照实验必须显式压预算。子任务预算仍来自 `spawn_subtask` 的参数，不继承这个旗标。argv 和那句忽略提示不进 Log；生效的预算写在清单的 `budget.total`。

## Testing

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、319 个测试）。覆盖 `--budget 6000` 的第一份清单 `budget.total`、`abc` 与 `0` 退出 2、passthrough 忽略并提示、`bench run` 的 argv 与 `results.json.budget`、passthrough 为 null、报表 `a`/`b` 带 `budget`。不打真网，不跑真机 bench。
