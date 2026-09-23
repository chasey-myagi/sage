# plugins 剩余重复与一词多义

## Problem

[共享守卫](./2026-09-24-shared-guards.md) 之后，plugins 与 apps 里还有几处同一事实写了多份，或同一个名字指两件事：

- 打分器摘要上限 `SUMMARY_LIMIT = 200` 与 `clampSummary` 在 `scorer/model.ts` 与 `compiler/score-common.ts` 各一份；`scorer/model.ts` 还自带一份 `TIERS` 集合，并用 `as` 收窄 tier。
- `compiler/stub.ts` 也有 `SUMMARY_LIMIT`，值是 80，指的是桩里那一行工具输入的长度，和打分器摘要不是一回事。
- tier 名字的守卫三份：`compiler/freeze.ts` 的 `isTier`、`compiler/format.ts` 的 `isTier`、`compiler/subtask-view.ts` 的 `isTierName`；`Tier` 类型本身在 core 与 `compiler/claim.ts` 各定义一次。
- 默认预算 `40_000` 两份：`apps/sage/src/coding-session.ts` 的 `DEFAULT_BUDGET` 与 `replay/offline.ts` 的 `REPLAY_DEFAULT_BUDGET`，注释说「和 CLI 默认一样」，靠人维持。
- `compiler/expand.ts` 直接写 `parentLeaf as EntryId`，而 core 已有品牌构造函数 `asEntryId`，只是没导出。

## Decision

纯重构，写进 Log 与模型可见的字节不变。

| 事实 | 唯一定义 | 删掉的副本 |
|---|---|---|
| 摘要上限与截断 | `compiler/score-common.ts` 的 `SUMMARY_LIMIT`、`clampSummary` | `scorer/model.ts` 的两份；该文件的 `TIERS` 改用 `isTier` 加 `SCORE_TIERS`，去掉一个 `as` |
| 桩里工具输入一行的长度 | `compiler/stub.ts` 的 `INPUT_LINE_LIMIT`（原 `SUMMARY_LIMIT`） | 无，只是改名 |
| tier 守卫 | `plugins/src/guards.ts` 的 `isTier` | `freeze.ts`、`format.ts`、`subtask-view.ts` 三份 |
| `Tier` 类型 | core `view/types.ts` | `compiler/claim.ts` 改为转出 core 的 `Tier`，包的公开导出不变 |
| 默认预算 | `compiler/compiler.ts` 导出 `DEFAULT_COMPILER_BUDGET`，经 `@sage/plugins` 公开 | apps 的 `DEFAULT_BUDGET`、offline 的 `REPLAY_DEFAULT_BUDGET` |
| EntryId 品牌 | core `log/types.ts` 的 `asEntryId`，新经 `@sage/core` 导出 | `expand.ts` 的 `as EntryId` |

摘要上限归编译器：截断是编译器对任何打分器回复的契约，model 打分器只是在指令里引用同一个数。默认预算归 plugins：它是编译器的参数，apps 与离线重放都是调用方。

## Alternatives considered

摘要上限放在 `scorer/types.ts`：那是纯类型文件，而且 jev、rule 打分器都不截断，截断发生在编译器。

默认预算留在 apps、由 plugins 反向引用：违反依赖方向。两边各留一个常量加注释：就是现状。

`isTier` 放进 core：core 读 Log 时不需要按 tier 名分支，放进去只为 plugins 服务。plugins 内已有 `guards.ts` 专放读 Log 的守卫。

## Consequences

`@sage/plugins` 新增导出 `DEFAULT_COMPILER_BUDGET`；apps 不再导出 `DEFAULT_BUDGET`（没有其他引用）。`@sage/core` 新增导出 `asEntryId`。`packages/plugins/README.md` 的离线重放一节写明 `budget` 缺省值出自这个常量。

`SUMMARY_LIMIT` 从此只指打分器摘要的上限。

## Testing

`pnpm check` 全绿（89 个测试文件、468 个测试），测试文件未改动。
