# 内核与 provider 的共享定义各留一份

## Problem

core 与 provider 里同一个事实写了好几份。`isRecord` 在 `log/types.ts`、`testing/replay.ts` 各有一份定义，`session.ts`（两处）、`view/passthrough.ts`、`testing/render.ts`、`view/assert.ts` 又把同一个判断内联手写，provider 的 `record.ts` 再写一遍。带 AbortSignal 的 sleep 在 `session.ts`（`defaultSleep`）与 `provider/src/retry.ts`（`delay`）逐字相同，退避公式 `base * 4 ** (n - 1)` 两处各写一次。全零 usage 有三份：`session.ts` 的 `EMPTY_USAGE`、`contract/collect.ts` 的 `EMPTY_USAGE`、`drive.ts` 的 `zeroUsage()`。「path 上最新的 view_epoch」写了四份：`session.ts` 的 `epochState` 与 `appendViewEpoch`、`view/passthrough.ts` 的 `latestEpoch`、`health.ts` 的主循环。`health.ts` 把用量四元组的类型抄了六遍、求 total 写了四遍，`view/types.ts` 又内联三遍。`log/repair.ts` 自己定义了一个与 provider 同名的 `ToolResultBlock`，理由是「provider 没导出」，这句话已经过时。

复制品各自演化，修 bug 只修到一份。

## Decision

共享定义放在依赖图里所有调用方都够得到的最低一层。core 与 provider 共同够得到的只有 `@sage/provider/contract`，所以跨两包的东西都进 contract：

| 定义 | 位置 | 替换掉的副本 |
|---|---|---|
| `isRecord` | `packages/provider/src/contract/record.ts`，经 contract index 导出，`@sage/core` index 转出 | `core/src/log/types.ts`、`core/src/testing/replay.ts` 的定义；`session.ts` 的 `assistantAfterLatestHeader` 与 `asHeader`、`view/passthrough.ts` 的 `isMessage`、`testing/render.ts` 的 `sortedReplacer`、`view/assert.ts` 的 `canonicalJson` 里的内联判断；`provider/src/record.ts` 的 `asRecord` 改为基于它 |
| `sleep`、`Sleeper`、`backoffMs` | `packages/provider/src/contract/backoff.ts` | `session.ts` 的 `defaultSleep` 与 `turnBackoffMs`；`retry.ts` 的 `delay`、`backoffMs` 与 `Sleeper`；`http.ts` 与 `session.ts` 里内联的 sleep 函数类型 |
| `emptyUsage()` | `packages/provider/src/contract/usage.ts` | `session.ts` 与 `collect.ts` 的 `EMPTY_USAGE`、`drive.ts` 的 `zeroUsage` |
| `TokenCounts`、`addUsage`、`inputTokens`、`totalTokens` | `packages/provider/src/contract/usage.ts`；`Usage` 改为 `TokenCounts` 加 `raw` / `details` | `session.ts` 的 `addUsage`；`health.ts` 的四元组类型、`addUsage` 与四处 total；`view/types.ts` 三处内联四元组 |
| `viewEpochs`、`latestViewEpoch` | `packages/core/src/log/epoch.ts`，`latestViewEpoch` 从 `@sage/core` 导出 | `session.ts` 的 `epochState` 与 `appendViewEpoch`（删掉 `seen ? latest : 0`）、`view/passthrough.ts` 的 `latestEpoch`、`health.ts` 主循环里的最高 epoch |
| `ToolResultBlock` | provider 的 `contract/message.ts`（本来就有） | `core/src/log/repair.ts` 的本地定义；`@sage/core` 不再转出这个类型 |

`health.ts` 的持久化字段名（`input`、`cacheRead`、`cacheCreation`、`output`、`total`）写进 `terminal.health`，不能改。内部改用 `TokenCounts` 累加，出口处由 `toHealthTokens` 换成这套名字，类型只定义一次，叫 `HealthTokens`。

退避参数不合并：provider 仍是 `DEFAULT_RETRY`（1 秒、4 秒），core 仍是 `TURN_RETRY_BASE_MS`（2 秒、8 秒）。只有曲线和计时器共享。

`emptyUsage` 是工厂函数而不是常量：`collect` 返回的 usage 会被调用方持有、写进 Log，共享一个对象会让一处修改波及所有人。

`session.ts` 原来动态 `import("@sage/provider/contract")` 取 `collect`，理由是 `scripts/gen-log-manifest.mjs` 用裸 Node 加载 `packages/core/dist`，contract 的出口是 TypeScript 源码。现在 core 静态引用 contract 的值，这个绕法没意义了。改为在生成脚本里注册一个解析钩子：相对导入的 `./x.js` 找不到时换成 `./x.ts`，类型交给 Node 的类型擦除。`session.ts` 的 `collect` 随之改为静态导入，`log/kinds.ts` 文件头里「不要值导入 provider」那句也改成「只导入 contract 子路径」。

## Alternatives considered

sleep 放进 core、provider 反向依赖 core：违反分层，provider 在 core 之下。各留一份再加注释互指：违反「一个事实一个定义」。

contract 的承诺是「不做 I/O、不含 HTTP」。计时器不是 I/O，`sleep` 无状态、不依赖任何模块，放进 contract 不破这条承诺；contract index 的文件头写明唯一的副作用是 `sleep` 里的计时器。

生成脚本改为只加载 `dist/log/registry.js` 与 `dist/log/kinds.js`：`kinds.ts` 自己也要用 `isRecord`，照样要解析 contract，问题没有消失。让 contract 另出一个编译好的出口：要给 provider 加第二套构建产物和导出条件，比一个十几行的钩子重。

`latestViewEpoch` 放进 view 目录：它读的是 Log 条目，与视图编译无关，所以放在 `log/`。

## Consequences

`@sage/core` 新增导出 `isRecord`、`latestViewEpoch`、`HealthTokens`，删掉 `ToolResultBlock`（仓内无人从 core 取它）。tools、plugins、apps 里复制的 `isRecord` 与 view_epoch 扫描这次没动，后续小 PR 换成这里的导出。

`scripts/gen-log-manifest.mjs` 依赖 Node 默认开启的类型擦除（22.18 起）。更老的 Node 22 跑这个脚本会失败，`pnpm check` 在那里会红。

`viewEpochs` 只收有限数。原来 session 与 passthrough 收任何 number（含 NaN），health 只收有限数。Log 的两个 store 在写入和重开时都用 `view_epoch` 的校验器拒绝非有限数，所以只有手拼的 path 才会看到差别。

## Testing

`pnpm check` 退出码 0：lint、tsc、分层自检、README、log manifest、87 个测试文件、418 个测试。没有改任何断言，也没有删用例；测试文件一个都没动。生成脚本的回归由 `scripts/__tests__/gen-log-manifest.test.ts` 覆盖：它用裸 Node 子进程跑脚本，在 core 静态导入 contract 之后，不加钩子时这条会红（`ERR_MODULE_NOT_FOUND ... contract/backoff.js`），加上钩子后变绿。
