# `sage stats --turns`：逐轮审计会话 Log

## Problem

`sage stats` 只给整次运行的 `runHealth` 和影子对账，看不出编译器在哪一轮做了什么：哪一轮缓存掉了、预算用到多少、各档位多少块、同一 epoch 里有没有块翻档、阶段决策为什么重开或不重开。#132 的对照分析要的正是这些逐轮数字。

开发机上一直用一份临时脚本 `audit.mjs` 回答这些问题，[9/24 状态文档](../../status/2026-09-24-sage-status.md) §4 把它列为未入库的产物。它自己 `JSON.parse` 每一行，不过 Log 校验；字段读法与 `runHealth` 各写一份（比如命中率的分母）；只打印人读的表格，不能接 `jq`。

同一节列出的 `compare.mjs` 不收编：它的准备任务和判分已经由 `sage bench run` / `bench report` 覆盖，剩下的是调第三方 runner，不在本仓范围。

## Decision

给 `sage stats` 加一个 `--turns` 开关，不新开子命令。输入相同（一个会话文件），读法相同（`JsonlLogStore.path`），退出码相同，只是输出换成一行 JSON `{ turns, invariants }`。

- 一轮 = 一条 `request_header` 加上它之后的第一条 assistant 回复，与 `replay` 的 `sliceTurns` 同一个切法；没有回复的请求不出行。
- 每轮的 token 与命中率直接对单条 assistant 调 `runHealth([entry])`，口径和整次运行的 `runHealth` 是同一个定义，不另写一份。
- 档位用 `isTier` 收窄。它原本在 `plugins/src/guards.ts`，而 `Tier` 类型在 core 的 `view/types.ts`，本次把守卫下沉到类型旁边，经 `@sage/core` 导出，plugins 的 4 处调用改从 core 引用，删掉 plugins 那份。请求头用 `requestHeaderOf`，manifest 字段的类型取自 core 的 `Manifest`。chunk 行、budget 和 `epochDecision` 逐字段守卫，坏行跳过、坏字段读作 null。
- 翻档只在前后两轮属于同一 epoch 时才算；跨 epoch 改档是重开本身，不算。
- 输出与 `sage stats` 的契约一致：只有计数、块 id、tool_use id、工具名和档位。任何 `reason` 字符串都不进输出：块的 `provider_error:<message>` 会带上外部文本；`epochDecision.reason` 在 `--ladder` 多 provider 时会经 `appendReason` 拼上 `sensitive:<绝对路径>`。`epochDecision` 只输出结构化字段 `rebuild`、`oldTokens`、`newTokens`、`remainingTurns`、`paybackTurns`。消息文本、工具输入和路径也不进。
- `stats` 的参数改由 `options.ts` 的 `readFlags` 解析。旗标表 `STATS_FLAGS` 由 `stats.ts` 自己持有，与 `bench/args.ts` 的 `BENCH_RUN_FLAGS` 对称；`args.ts` 的总 usage 调 `statsUsage()` 渲染这一行；参数错误时 `stats` 只打印自己的 usage 行，不 import `args.ts`，两边没有循环依赖。

## Alternatives considered

新开子命令 `sage stats-turns` 或 `sage audit <session.jsonl>`：输入、读法、退出码和隐私契约跟 `sage stats` 完全一样，单开一个子命令只会让 `main.ts` 多一个分支、usage 多一行，两份参数校验还要保持对称。`stats` 现在读一个会话、出一份报告，`--turns` 只是换一种粒度，作为开关更贴近现有形状。

输出每轮一行 JSON（JSONL），最后一行放汇总：两种形状混在同一个流里，消费者得靠字段猜是哪一种。`sage stats` 和 `bench report` 都打印单个 JSON 对象，这里保持一致，要逐行看时用 `jq -c '.turns[]'`。

导出 `sliceTurns` 复用它切轮：它给出请求头和回复，但不带该轮的 manifest，要拿到还得按下标回扫，比在一次遍历里顺手收集更绕。这里沿用同一个切法，并在 README 写明口径。

兼容旧格式（缺 `request_header.contextWindow` 的会话）：预发布不留兼容层，旧文件与 `sage stats` 一样报 `Log corrupted` 并退出 1。

## Consequences

`@sage/core` 多导出一个 `isTier`，plugins 不再有自己的一份。不输出 `epochDecision.reason`，想看重开原因的人要回 Log 原文看；把原因改成结构化字段属于 plugins 的格式改动，不在本次范围。`sage stats` 的 usage 行变成 `sage stats [--turns] <session.jsonl>`。原来手写的 argv 检查换成 `readFlags`，`-x` 之类的未知旗标仍打印 usage 并退出 1，行为不变。manifest 字段改名时，这里的守卫会把该字段读成 null 或 0，而不是报错；`check:log-manifest` 和 README 的口径表是发现漂移的地方。

## Testing

`apps/sage/test/stats-turns.test.ts` 用 `@sage/core/testing` 的 faux adapter 和 `pathlessTool` 跑一个真实 `AgentSession`，写进 `JsonlLogStore`；视图在 passthrough 上按轮换成预先排好的 manifest（4 轮，第 4 轮开 epoch 1），之后再追加一条悬空 tool_use 和一条孤儿 tool_result。断言面只有 `runCli(["stats", "--turns", file])` 的 stdout：

- 每轮的 tokens、命中率（含分母为 0 的 null）、budget、档位计数、tools、epochDecision（有与无，`paybackTurns` 为 null 与为数字）；
- 第 3 轮同 epoch 内两块翻档，第 4 轮跨 epoch 改档不算；
- invariants 的配对（`unpairedToolUses: ["dangle"]`、`orphanToolResults: ["ghost"]`）、错误数（调用未注册工具得到的 1 个错误）、条目数、models、outcome 和 health；
- stdout 不含任务文本、回复文本、块 reason、错误文本和临时目录路径；第 4 轮的决策 reason 写成 `payback:1<=3;route:sensitive:/work/.env`，stdout 不含 `/work/.env`（评审指出后补的用例，在输出 reason 的版本上为红）；
- 只写 `--turns` 不给文件、给两个文件、写成 `--turns=yes`，都打印含 `sage stats [--turns] <session.jsonl>` 的 usage 并退出 1。

实现前 4 条用例里 3 条红灯（usage 不认 `--turns`）；隐私用例当时因 stdout 为空而空过，已补上 `code === 0` 和 stdout 含 `"turns"` 的前置断言。`pnpm build` 后对 `packages/plugins/test/replay/fixtures/issue-63/budget8000-vue10874-36turns.jsonl` 跑 `node apps/sage/bin/sage.mjs stats --turns`，36 轮、44 对 tool_use / tool_result 全配对；开发机上的 9/23 真机轨迹是旧格式，读不了，退出 1。
