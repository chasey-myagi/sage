# 「head」一词多义：持久化字段改名提议

## Problem

`head` 在仓里指三样东西：

- `Head` 类型与 `Request.head` / `CompiledView.head` / `ViewContext.head`：system prompt 加 tool schema，请求里排在消息之前的那段。`CacheBreakpoint` 的 `"head"`、`headHash`、`HeadChangedMidEpochError` 都是这个意思。
- `view_epoch` 条目的 `details.epochHead`：开启这个阶段的视图的第一条消息（编译器放冻结 claim 的那条）。core 的 `AgentSession.appendViewEpoch` 与 plugins 的 `replay/offline.ts` 写它，plugins 的 `compiler.ts`、`rebuild.ts` 读它。
- manifest 的 `calibration.head`：固定开销的 token 数，等于阶段第一轮实测输入减去那一轮的 `budget.used`。plugins 的 `calibrate.ts` 写和读，core 的 `log/kinds.ts` 校验它是有限数。

AGENTS.md「一个词一个意思」拿这个当反例。后两个是写进 Log 的字段名，jsonl 里已经有大量历史记录（包括 `packages/plugins/test/replay/fixtures/` 下的回放夹具），改名要同时处理读旧 Log，不能在一个纯重构 PR 里顺手改。

`refactor/core-structure` 已经做了不碰 Log 的部分：`session.ts` 里指 `messages[0]` 的局部变量改名为 `firstMessage`；`Head` 类型、`ViewEpochPayload`、`CalibrationReport.head` 的 JSDoc 写清了各自含义，并指向本文。

## Proposal

`head` 只保留第一个意思：system prompt 加 tool schema。另外两个持久化字段改名：

| 现名 | 新名 | 含义 |
|---|---|---|
| `view_epoch.details.epochHead` | `view_epoch.details.prefixMessage` | 开启阶段的视图的第一条消息，即冻结前缀里的那条消息 |
| `manifest.calibration.head` | `manifest.calibration.overheadTokens` | 固定开销 token 数：阶段第一轮实测输入减去该轮 `budget.used` |

做法（首个 tag 之前，按 G4 不留兼容层）：

1. 一个 PR 同时改写入方（core `appendViewEpoch`、plugins `offline.ts`、`calibrate.ts`）、读取方（`compiler.ts`、`rebuild.ts`、`calibrate.ts`）、core `log/kinds.ts` 的校验与 `CalibrationReport` 类型。
2. 同一个 PR 用一次性脚本把仓内所有 jsonl 夹具里的这两个字段改名，重新生成 `expected/`。脚本不进仓，PR 描述贴出命令与改动行数。
3. 仓外已有的 Log（bench 结果目录）不迁移：读到旧名时 `rebuild` 找不到冻结前缀，`calibrate` 取不到固定开销，按缺失处理（前者不补前缀消息，后者 `head` 记 0）。PR 在说明里写清旧 bench Log 离线重放会与原始运行出现差异。

## What we give up

- 旧 bench Log 的离线重放不再逐字节复现：冻结前缀消息与校准固定开销会丢失。要复现旧结果，得用改名之前的提交。
- 一次涉及 core、plugins、夹具的大 diff，与并行中的编译器重构冲突概率高，要排在那批改动合入之后。

## Acceptance criteria

- `rg -n '\bepochHead\b|calibration\.head\b' packages apps bench` 无结果；`rg -nw head packages/*/src` 的每一处都指 system+tools。
- `pnpm check` 全绿；plugins 的 issue-46 / issue-50 回放断言不改，只换夹具字段名。
- `docs/log-manifest.md` 若 `doc` 文本变化则同步生成。

## Risks

- 夹具改名脚本漏掉嵌套位置（`details` 在 view_epoch 条目上，`calibration` 在 manifest payload 上），回放测试会以「阶段前缀缺一条消息」的形式失败，而不是字段缺失的报错，排查成本高。脚本应按条目 kind 精确改写，不做全文替换。
- 若有外部消费者直接读 jsonl（例如 bench 的统计脚本），它们会静默读到缺失值。改名前先 `rg` 一遍 `bench/` 与 `apps/sage` 的 stats 代码。
