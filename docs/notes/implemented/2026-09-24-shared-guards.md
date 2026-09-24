# tools、plugins、apps 换用共享定义

## Problem

[内核共享定义](./2026-09-24-core-helpers.md) 把 `isRecord`、`sleep`、`emptyUsage`、`inputTokens`、`latestViewEpoch` 收成一份之后，tools、plugins、apps 里的副本还在：plugins 的 `src` 有 12 个文件各写一遍 `function isRecord`，其中 `log-read.ts` 与 `task-input.ts` 还把自己那份导出给别的文件用；`apps/sage/src/guards.ts` 再写一份。「取错误消息」的 `error instanceof Error ? error.message : String(error)` 有三个具名定义（`tools/src/input.ts`、`core/src/hooks/dispatcher.ts`、`plugins/src/subtask/spawn.ts`）和七处内联。「路径上最新的 view_epoch」在 plugins 里又写了三遍，全零 usage 字面量三份，jev 打分器自己写了一个 `delay`。

测试替身方面，`packages/core/test/support/faux.ts` 只剩转出，apps 与 plugins 的 8 个测试文件还从这个路径导入，与「测试替身只有一套：`@sage/core/testing`」相违。

`apps/sage/src/io.ts` 从 `bench/types.ts` 取 `BenchDeps`，`bench/types.ts` 又从 `io.ts` 取 `CommandIo`，两个文件互相 import。

`scripts/gen-log-manifest.mjs` 依赖 Node 22.18 起默认开启的类型擦除，根 `package.json` 的 `engines.node` 仍写 `>=22`。

## Decision

纯重构，写进 Log、报告与 stderr 的内容逐字节不变。

| 共享定义 | 位置 | 替换掉的副本 |
|---|---|---|
| `isRecord` | `provider/src/contract/record.ts`，`@sage/core` 转出 | 13 个 src 定义：plugins 的 `scorer/model.ts`、`scorer/jev.ts`、`replay/offline.ts`、`compiler/expand.ts`、`compiler/rebuild.ts`、`compiler/freeze.ts`、`compiler/log-read.ts`（导出）、`compiler/cache.ts`、`compiler/calibrate.ts`、`subtask/task-input.ts`（导出）、`claims/progress-ledger.ts`、`claims/agents-md.ts`，以及 `apps/sage/src/guards.ts`；9 个测试文件里的辅助定义（plugins 6 个、apps 3 个）；`tools/src/input.ts` 的 `asRecord` 改为基于它，去掉一个 `as` |
| `errorText` | 新文件 `provider/src/contract/error.ts`，`@sage/core` 转出 | 3 个定义：`tools/src/input.ts`、`core/src/hooks/dispatcher.ts`、`plugins/src/subtask/spawn.ts`；7 处内联：`core/src/session.ts`、`plugins/src/compiler/assemble.ts`、apps 的 `main.ts`、`stats.ts`、`print.ts`、`bench/report.ts`、`bench/run.ts` |
| `latestViewEpoch` | `core/src/log/path.ts`，本来就从 `@sage/core` 导出 | 3 处：`compiler/freeze.ts` 的导出 `currentEpoch`（`subtask-view.ts` 改用 core 的）、`compiler/calibrate.ts` 的 `currentEpoch`、`replay/offline.ts` 的 `commitView` 里的扫描 |
| `emptyUsage` | `provider/src/contract/usage.ts` | 3 处：`subtask/spawn.ts` 与 `compiler/score-common.ts` 的 `EMPTY_USAGE`、`scorer/jev.ts` 的 `usageFrom` 里的零字面量 |
| `sleep`、`Sleeper` | `provider/src/contract/backoff.ts` | `scorer/jev.ts` 的 `delay` 与两处内联的 sleep 函数类型 |
| `inputTokens` | `provider/src/contract/usage.ts` | `apps/sage/src/print.ts` 进度行里的三项求和 |

`errorText` 的层级按调用方定：core 的 dispatcher 与 session 要用它，所以不能放在 core 之上；tools 只依赖 core，所以要经 `@sage/core` 转出。放进 contract，与 `isRecord` 同路，core 只多转出一个名字。

`plugins/src/compiler/score-shadow.ts` 原来也叫 `errorText`，但语义不同：空消息退回 `String(error)`，再不行写 `"error"`，保证影子打分的 `fallback` 字段不为空。它改名为 `shadowFailureReason` 并在 JSDoc 写明差异，不并入共享定义。

测试替身：8 个测试文件改为从 `@sage/core/testing` 导入（`rebuild.test.ts` 里的别名 `MODEL` / `SYSTEM` 改用原名 `FAUX_MODEL` / `FAUX_SYSTEM`），删除 `core/test/support/faux.ts`。`core/test/support/canonical.ts` 保留：它是刻意不依赖 core 的独立预言机，不是替身。

循环依赖：`TextOut` 与 `CommandIo` 挪到新文件 `apps/sage/src/command-io.ts`。`io.ts` 与 `bench/types.ts` 都从它取，`io.ts` 仍单向引用 `bench/types.ts`。

Node 版本：根 `package.json` 的 `engines.node` 改为 `>=22.18`，`docs/development.md` 工具链表与 `README.md` 快速开始同步。

## Alternatives considered

`errorText` 放在 tools 或 plugins：core 的 dispatcher 够不到，内核还得留一份。放在 core 自己的文件：provider 以外的所有层都够得到，但 core 已经约定共享的纯函数住在 contract，再开一处要多记一条规则。

jev 的退避改用 `backoffMs`：jev 的曲线是 `500 × 2^n`，`backoffMs` 是 `base × 4^(n-1)`，换过去就改了重试间隔，不是重构。保留原曲线，只共享计时器。jev 的 `sleepImpl` 也没有传 `signal`，原来就不传，传了会让取消时提前醒来，属于行为修改。

`calibrate.ts` 与 `offline.ts` 的 `inputTotal` 读的是 Log 里未经校验的 usage，逐项做有限数检查，和 `inputTokens`（输入已是 `TokenCounts`）不是同一件事，没有替换。

`io.ts` 与 `bench/types.ts` 合并：`BenchIo` 与 `CliIo` 各自服务不同子命令，合在一起只是把循环藏进一个文件。

## Consequences

`@sage/core` 新增转出 `errorText`。`@sage/provider/contract` 新增 `errorText`。plugins 不再导出 `isRecord`（`log-read.ts`、`task-input.ts`）与 `currentEpoch`（`freeze.ts`），这些都不在包的公开入口上。

CI 的 `.github/workflows/ci.yml` 写的是 `node-version: 22`，setup-node 会解析到 22 的最新版（高于 22.18），不用改。本地用更老的 22.x 会在 `pnpm check:log-manifest` 失败，现在 `engines` 会先提示。

仍留在原处、这次没动的重复：`finite`（`calibrate.ts`、`offline.ts`、`cache.ts` 与两个回归测试）、`isMessage`（`offline.ts`、`rebuild.ts`、`log-read.ts` 等）、`epochHeadFor`（`log-read.ts` 与 `rebuild.ts`，差一个 `structuredClone`）、按 epoch 找区间的扫描（`calibrate.ts` 的 `epochRange`、`log-read.ts` 的 `epochManifest`、`freeze.ts` 的 `manifestsInEpoch`）。它们都在 plugins 内部，下沉位置要另议。`core/test/testing/replay.test.ts` 里有一处内联的错误消息写法，属于 core 测试自身，未改。

## Testing

`pnpm check` 退出码 0：lint、tsc、分层自检、README、log manifest、87 个测试文件、419 个测试，与重构前相同。测试文件只改了 import 与删掉本地的 `isRecord` 辅助函数，没有改断言，没有删用例。
