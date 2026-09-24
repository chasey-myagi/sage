# 「head」一词多义：持久化字段改名

## Problem

`head` 在仓里指三样东西：

- `Head` 类型与 `Request.head` / `CompiledView.head` / `ViewContext.head`：system prompt 加 tool schema，请求里排在消息之前的那段。`CacheBreakpoint` 的 `"head"`、`headHash`、`HeadChangedMidEpochError` 都是这个意思。
- `view_epoch` 条目的 `details.epochHead`：开启这个阶段的视图的第一条消息（编译器放冻结 claim 的那条）。
- manifest 的 `calibration.head`：固定开销的 token 数，等于阶段第一轮实测输入减去那一轮的 `budget.used`。

AGENTS.md「一个词一个意思」拿这个当反例。后两个是写进 Log 的字段名，仓内回放夹具里也有，所以没有在 `refactor/core-structure` 那次纯重构里顺手改。那次只改了不碰 Log 的部分，并在 JSDoc 里指向本文。

## Decision

`head` 只保留第一个意思：system prompt 加 tool schema。另外两个持久化字段改名，首个 tag 之前按 G4 不留兼容层：

| 旧名 | 新名 | 含义 |
|---|---|---|
| `view_epoch.details.epochHead` | `view_epoch.details.prefixMessage` | 开启阶段的视图的第一条消息，即冻结前缀里的那条消息 |
| `manifest.calibration.head` | `manifest.calibration.overheadTokens` | 固定开销 token 数：阶段第一轮实测输入减去该轮 `budget.used`，下限 0 |

写入方：

- core `AgentSession.appendViewEpoch`（`session.ts`）写 `details.prefixMessage`。
- plugins `replay/offline.ts` 的 `commitView` 写 `details.prefixMessage`。
- plugins `compiler/calibrate.ts` 的 `calibrationFor` 写 `overheadTokens`；`Calibration` 类型同名字段。

读取方：

- plugins `compiler/log-read.ts` 的 `prefixMessageFor`（原 `epochHeadFor`，`assemble.ts` 的 `reusablePrefixMessage` 用它）与 `compiler/rebuild.ts` 的同名私有函数读 `details.prefixMessage`。
- plugins `compiler/calibrate.ts` 的 `storedCalibration`（`expectedFrozenPrefix` 用它）读 `calibration.overheadTokens`；`compiler/epoch.ts` 的 `projectionFor` 读 `Calibration.overheadTokens`。

校验与类型：core `log/kinds.ts` 的 `isCalibration` 校验 `overheadTokens` 为有限数或缺省；`CalibrationReport.head` 改为 `overheadTokens`；`ViewEpochPayload`、`ManifestPayload`、`Manifest.calibration` 的 JSDoc 同步。`calibrate.ts` 内部的 `anchorHead` / `firstHead` / `latestHead` / `headFromPairs` 改为 `…Overhead`，文件头与 `compiler.ts`、`format.ts`、`rebuild.ts`、`assemble.ts` 的文档同步；`packages/plugins/README.md` 的 `cache.expectedReadTokens` 与离线重放两段同步。

夹具：一次性脚本（不进仓）逐行解析 `packages`、`apps`、`bench` 下全部 `*.jsonl`，只改 `kind === "view_epoch"` 条目的 `details.epochHead` 与 `kind === "manifest"` 条目的 `payload.calibration.head`，原位改键名、保持键序，改写的行先确认 `JSON.stringify(JSON.parse(line)) === line`。结果 3 个文件、37 行：

- `packages/plugins/test/replay/fixtures/issue-46/long-deepseek-flash.jsonl`：1 行（`view_epoch`；这份夹具的 `calibration` 早于 `head` 字段，没有可改的）。
- `packages/plugins/test/replay/fixtures/issue-50/long-deepseek-flash.jsonl`：1 行（同上）。
- `packages/plugins/test/replay/fixtures/issue-63/budget8000-vue10874-36turns.jsonl`：35 行（`calibration.head`；没有 `view_epoch`）。

逐文件把新键名换回旧键名后与改前逐字节相同，diff 只有字段名。`packages/core/test/replay/*/expected/` 里没有这两个字段，没有重新生成任何期望产物，没有用 `UPDATE_EXPECTED`。`docs/log-manifest.md` 重新生成后无变化（`view_epoch` 与 `manifest` 的 `doc` 文本没有提到这两个字段）。

测试：回放测试（issue-46 / issue-50 / issue-63）断言不改。直接读写这两个字段的用例只换字段名：`packages/core/test/session/epoch.test.ts` 读 `details.prefixMessage`；plugins 的 `compiler.test.ts`、`decide.test.ts`、`scorer/compiler.test.ts`、`regressions/issue-50.test.ts`、`regressions/issue-63.test.ts` 手写 `view_epoch` 时用新键；`compiler/calibrate.test.ts` 读写 `overheadTokens`，四个用例标题里的 head 改为 overhead，期望值不变。

## Alternatives considered

读旧名时回退：就是兼容层，G4 不许。

`overheadTokens` 叫 `overhead`：其他 token 数字段都带 `Tokens` 后缀（`frozenTokens`、`expectedReadTokens`），跟随惯例。

`prefixMessage` 叫 `firstMessage`：`session.ts` 里的局部变量已经叫这个名字，但落进 Log 后读者更关心它在冻结前缀里的角色，不是位置。

全文替换 `"epochHead"` / `"head"`：`"head"` 在夹具里还是 `cacheBreakpoints` 的取值，全文替换会改坏它们。

## Consequences

仓外已有的 Log（bench 结果目录、旧会话）不迁移。读到旧名按缺失处理：

- 旧 `view_epoch` 没有 `details.prefixMessage`：`rebuildFromLog` 找不到冻结前缀消息，重建时不补这条消息；编译器在阶段内取不到已存的前缀消息，退回重新渲染 epoch-head 消息。
- 旧 manifest 的 `calibration` 没有 `overheadTokens`：`expectedFrozenPrefix` 取固定开销 0。`calibrationFor` 从用量与 `budget.used` 重新计算，不读旧字段，不受影响。
- `log/kinds.ts` 对 `calibration` 的字段都是可选有限数，旧的 `head` 属于不检查的其他字段，旧 Log 仍能重开。

因此旧 bench Log 的离线重放不再逐字节复现原始运行：冻结前缀消息与 `cache.expectedReadTokens` 里的固定开销会丢失。要复现旧结果，用改名之前的提交。

仓内回放用例在夹具改名前后都通过：它们的断言不依赖这两个字段的取值，所以夹具改名是为了让仓内 Log 只有一种形状，不是为了让测试变绿。

`packages/provider/src/contract/message.ts` 的 `Head` JSDoc 与 `docs/notes/implemented/2026-09-24-core-structure.md` 曾链接本文在 `docs/notes/proposed/` 下的旧路径，已在同一批改动里改为新字段名与 implemented 路径。

## Testing

`pnpm check` 退出码 0（88 个测试文件、436 个测试）。`rg -n '\bepochHead\b|calibration\.head\b' packages apps bench scripts` 只剩上面 provider JSDoc 一处。
