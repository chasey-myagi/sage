# plugins 剩余的重复定义收成一份

## Problem

[tools、plugins、apps 换用共享定义](./2026-09-24-shared-guards.md) 之后，plugins 里还有四组重复：

- `finite`：`compiler/calibrate.ts`、`compiler/cache.ts`、`replay/offline.ts` 各一份，`test/regressions/issue-50.test.ts` 与 `issue-63.test.ts` 各一份。
- `isMessage`：`compiler/log-read.ts`（导出）、`compiler/rebuild.ts`、`compiler/calibrate.ts`、`replay/offline.ts` 各一份；core 的 `view/passthrough.ts` 还有一份同样的私有定义。
- 读 `view_epoch.details.prefixMessage`（[字段改名](./2026-09-24-head-naming.md) 前叫 `epochHeadFor`）：`log-read.ts` 与 `rebuild.ts` 各一份，只差一个 `structuredClone`。
- 按 epoch 找区间的扫描：`calibrate.ts` 的 `epochRange`、`log-read.ts` 的 `epochManifest`、`freeze.ts` 的 `manifestsInEpoch`，三份都是「找到开启该 epoch 的 `view_epoch`，走到下一个 `view_epoch` 为止」。

## Decision

纯重构，写进 Log 与视图的内容不变，测试断言一行没改。

| 共享定义 | 位置 | 替换掉的副本 |
|---|---|---|
| `epochSpan(path, epoch)` | core `log/path.ts`，`@sage/core` 导出 | `calibrate.ts` 的 `epochRange` |
| `manifestsInEpoch(path, epoch)` | core `log/path.ts`，`@sage/core` 导出，基于 `epochSpan` | `freeze.ts` 的 `manifestsInEpoch`、`log-read.ts` 的 `epochManifest`（改为取 `[0]`） |
| `prefixMessageFor(path, epoch)` | core `log/path.ts`，`@sage/core` 导出，返回克隆 | `log-read.ts` 的导出版与 `rebuild.ts` 的私有版；`rebuild.ts` 插入前不再另外克隆 |
| `isMessage` | core `log/path.ts`，`@sage/core` 导出 | plugins 4 份（`log-read.ts` 导出版、`rebuild.ts`、`calibrate.ts`、`offline.ts`）与 core `view/passthrough.ts` 1 份 |
| `finite` | 新文件 `packages/plugins/src/guards.ts` | `calibrate.ts`、`cache.ts`、`offline.ts` 与两个回归测试 |

合计删掉 15 份定义（plugins 源码 12 份、plugins 测试 2 份、core 1 份）。

`isMessage` 本来按分工该放在 plugins 的共享模块，但 core 的 `prefixMessageFor` 要用它，放在 plugins 里 core 就得再留一份。按「一个事实一个定义，放在所有调用方够得到的最低一层」放进 core；provider 的 contract 更低，但这次不碰 provider 包。

`epochSpan` 的 `start` 是开启 view_epoch 的下一条，`calibrate.ts` 原来的 `epochRange` 从 view_epoch 自己开始。固定开销的扫描只看 manifest 与 assistant 消息，多一条 view_epoch 不影响结果。未开启的非零 epoch，`epochRange` 返回 `{length, length}`，现在 `epochSpan` 返回 undefined，`calibrate.ts` 在调用处换成同样的空区间。

## Alternatives considered

`finite` 也放进 core：core 的 `health.ts` 有一份私有 `finite`，`log/kinds.ts` 有 `isFiniteNumber` / `isOptionalFinite`，apps 有 `isFiniteNumber`。把它们合成一份是跨三个包的事，而且最低一层是 provider 的 contract，这次不碰。

`manifestsInEpoch` 留在 plugins 的 `log-read.ts`：它是纯 Log 路径查询，与 `epochSpan` 放在一起，调用方只需要一个导入来源。

`prefixMessageFor` 不克隆、由调用方克隆：原来 `log-read.ts` 在查询里克隆，`rebuild.ts` 在调用处克隆。统一在查询里克隆，调用方不会忘，也改不到 Log 里的对象。

## Consequences

`@sage/core` 新增导出 `epochSpan`、`manifestsInEpoch`、`prefixMessageFor`、`isMessage`。plugins 不再从 `log-read.ts` 导出 `isMessage` 与 `prefixMessageFor`，这些都不在包的公开入口上。

仍留在原处的重复：core `health.ts` 的 `finite` 与 `isAssistant`、`kinds.ts` 的 `isFiniteNumber`、apps 的 `isFiniteNumber`；`calibrate.ts` 与 `offline.ts` 的 `isAssistant` 签名不同（一个收 entry，一个收 payload），没有合并。

## Testing

`pnpm check` 退出码 0（88 个测试文件、436 个测试）。测试文件只改了两处：两个回归测试删掉本地 `finite`，改从 `src/guards.ts` 导入。没有改断言，没有删用例。
