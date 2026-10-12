# Agent Note: sage calibrate-jev——用录好的会话离线校准 jev 阈值

## Problem

jev 打分器 v2 的四个阈值（`full` / `none` / `low` / `keep`）是按两种错误代价不对称拍的起点，没有在数据上调过。会话 Log 本身就是现成的标注集：每个工具块之后有没有被 `expand` 取回、被重读或被编辑，从 Log 就能算出来。缺的是一条路：把录下的块按线上的方式问 Jev，拿到概率，再在一组阈值上比较「省略了却又被需要」的错误和多留原文的花费。这条路要花钱，所以必须先估算、可封顶、可重跑不重复计费。

## Decision

- **新子命令 `sage calibrate-jev <session.jsonl…> --out <dir> [--max-usd N] [--dry-run] [--t-full …] [--t-none …] [--t-low …] [--t-keep …]`**，只调 Jev，不调主模型，不写会话。
- **请求与线上入口模式逐字节一致。**`@sage/plugins` 新增 `entryJevRequests(path)`（`src/replay/jev-entry.ts`）：按 `sliceTurns` 走轮次，每轮用 passthrough 投影该轮前缀、`indexChunks` 建块，块第一次出现、不是 `expand`、不少于 `DEFAULT_MIN_SCORE_TOKENS` 时调 `chunkRequest`。原文与 assistant 文字取自投影（线上入口打分看到的也是尚未定档的原文），`task` 是前缀的 `latestQuery`，`ledger` 来自 `rebuildFromLog` 重建的该轮请求。POST 的字符串由 `jev/state.ts` 的 `requestBody` 生成，jev 打分器也从那里导入，两边只有一份请求体定义。
- **发送池只有一份。**worker pool 与熔断从 `jev.ts` 下沉为 `jev/http.ts` 的 `postMany(bodies, endpoint, hooks)`：4 个在途，单个请求失败只占自己那一格、其余照发，429 / 529 重试用尽才熔断，`signal` 中止后不再开新请求。jev 打分器的 `postRequests` 与校准都调它。校准经 `hooks.shouldStop` 卡预算，经 `hooks.onPosted` 在每个回答到达时写缓存与花费。`postSystemOne`、`PostTarget`、默认地址与并发上限都留在 `http.ts` 里，不从包入口导出；`MODEL_ID` 在 `jev.ts` 里改名为 `JEV_MODEL_ID`，包入口直接转出，不起别名。
- **校准的失败语义。**单个请求失败只计入 `failed`，其余照发：一个总失败的请求不会让每次重跑都卡在同一位置。熔断时剩下的计入 `unsent`，`stopped` 为 `rate-limited`。有失败或熔断时结果照常写出，退出 1。
- **缓存键是请求体的 sha256**，文件在 `<out>/cache/<hash>.json`，只存 `answers` 和用量。`JEV_MODEL` 在请求体里，换模型自然是新键；多条会话问出相同请求体时只发一次。
- **预算闸门。**估算用 `estimateRequestTokens`：state 加全部问题，每 2.5 个 UTF-8 字节算一个 token，与打分器控制长度的估算同源，偏多不偏少。单价不另写数字，运行时读内置目录 `typesafe/jev-latest` 一行的 `inputPerMTok`（`jevInputUsdPerMTok`，出处是该行的 `source`，TypeSafe 发布 System One 的公告）；只在 `calibrate-jev` 里按需读取，目录缺这一行时只有这个子命令报错。`--max-usd`（默认 1 美元）是跨次累计上限：`spend.json` 已有花费加本次估算超过它就拒绝执行，退出 2；执行中累计实际花费一旦超过，不再发新请求，在途的收完，结果照常写出，退出 2。每个请求开始前才检查，越线时最多 4 个在途，超支上界是 4 个请求的费用。
- **标签。**`stats.ts` 的 `neededLaterIn(entries)` 组合 `expandedLater` 与 `touchedLater`：后者对 `read` 结果判断同一 `path` 在更后的轮次是否又被 `read`（重读）或 `edit`（编辑）。影子对账与校准共用这一份。判定点 `at` 是该轮视图提交的起点，`turn` 是该轮清单的 `turn`；compiler 与 passthrough 会话都会写清单。
- **扫描**在 `calibrate-sweep.ts`，纯函数：每组阈值把缓存的回答交给打分器自己的 `decideTier`，summary 与 none 都算省略。每行输出各档数量、`needed`、`omittedNeeded`、漏判率 `missRate = omittedNeeded / needed`、`precision`、`recall` 和期望成本。成本与编译器阶段决策同一套输入价等价 token 口径（`ratesFor`）：`read = cacheRead / input`，`write = max(cacheWrite, input) / input`，缺定价时用默认的 0.1 / 1.25；省略后又被需要按 `tokens × write` 计一次取回，保留 full 却从未用到按 `tokens × read × 剩余轮数` 计。这样不同币种和没有内置定价的会话可以相加，汇总里的 `unpriced` 数出按默认费率计的块。推荐取成本最低的一组，平局取离 `DEFAULT_JEV_THRESHOLDS` 最近的一组。
- `sweep.json` 的每个块带 `session`（会话文件名），块 id 跨会话不唯一。
- `--max-usd` 与阈值列表的校验进 `options.ts` 共用参数表（`checkUsd`、`checkProbabilities`），写错退出 2，与其他数值旗标同一口径。

## Alternatives considered

- **整段重编译（`replayOffline`）再拿编译器的视图当 `prevRequest`**：离线编译会用另一套预算、没有申领单，得到的视图与线上不同，ledger 也对不上。录下的清单与 `claimContent` 已经能重建当轮请求，直接用它更接近线上。
- **只问线上清单里仍是 full 的块**（线上入口候选的完整条件）：线上已被预算裁掉的块就永远没有标签。离线把所有达到门槛的块都问一遍，代价只是多几个请求，README 写明了这一差别。
- **用 `listedChunkIds`（上一条清单）判断块是不是新的**：依赖清单的写法；改用回放过程中自己记的已见集合，不读清单也能判断。
- **任一请求失败就停发**：一个总失败的请求每次都让校准停在同一位置，后面的块永远拿不到回答。只有限流熔断值得停：继续发只会继续被拒。
- **校准自己写一份 worker pool**：与打分器的并发、熔断语义会慢慢分叉。下沉到 `http.ts` 后两边是同一份代码。
- **成本按各会话定价的币种直接相加**：币种混用时数字没有意义，缺定价的会话会记 0、显得最便宜。换成输入价等价 token 与编译器的阶段决策同一口径。
- **平局取网格最前**：网格最前是各阈值最小的一角，也就是最激进省略的一组。离默认值最近的一组改动最小。
- **单价直接写 0.042**：目录里已经有这一行和出处，第二份数字迟早不一致。
- **按真实分词器估算**：要引入分词依赖，且 Jev 的分词并不公开。保守估算只会让闸门更早拒绝。

## Consequences

- 校准结果只说明「省略之后是否被取回、重读或编辑」，看不到模型因为缺原文而走错但没去取回的情况；`precision` 偏乐观。
- passthrough 会话没有申领单，请求里的 `ledger` 恒为空串，Jev 的输入分布与 compiler 会话不同。passthrough 从不省略，只能靠重读与编辑打上「需要」，它的需要率是下界。
- `cost` 没有计入 expand 多出的那一轮模型调用、取回文本在之后各轮的缓存读，以及取回时实际写价与写价的差异；summary 笔记的 token 也不计。
- jev 打分器全部失败时的 `fallback` 取请求顺序里第一个失败的原因，不再是完成顺序里的第一个。
- `sage stats` 的 `laterNeeded` 也多算了编辑这一信号。
- 新增的包导出（`entryJevRequests`、`postMany`、`requestBody`、`estimateRequestTokens`、`readJudgment`、`decideTier`、`DEFAULT_JEV_THRESHOLDS`、`ratesFor`）里，前几项只有校准这一个生产调用方。
- 数据与结果不进仓库，`--out` 由使用者指定。

## Testing

- `apps/sage/test/calibrate-jev.test.ts` 先用 faux provider 加 `--scorer jev --scorer-apply entry` 真跑 `sage -p`（含两次 `update_ledger`、六次 `read`，其中 `a.ts` 读两次），用 `jsonFetch` 记下线上 Jev 请求体，再对生成的会话跑 `calibrate-jev`：
  - 发出的请求体与线上逐个相等（把 ledger 置空或把 assistant 文字置空，这个用例都会红）；
  - 第一次读的 `a.ts` 被标为需要；passthrough 会话里重读与之后的 `edit` 都打上「需要」，所有请求的 `ledger` 为空串；
  - 重跑全部命中缓存，0 次请求，`spend.json` 不变；两条会话一起校准时每块带各自的 `session`，相同请求体只发一次；
  - `--dry-run` 不发请求、不写 `spend.json`、不需要 key；`--max-usd 0` 与 `spend.json` 里已有的花费都会让闸门拒绝，退出 2；
  - 回答远超上限时，4 个在途请求收完后不再发第 5 个，已有结果照常写出，退出 2；
  - 一个请求 500 时其余照发，`failed` 为 1，退出 1；全部 429 时 4 个在途请求各重试 3 次后熔断，剩下 2 个不发；
  - 扫描行的档位、`needed`、`missRate`、`precision`、`recall` 与按费率手算的 `cost` 一致，平局时推荐离默认值最近的一组；缺定价时按默认写价 1.25 计并计入 `unpriced`。
- `packages/plugins/test/scorer/` 的 jev 用例（含部分失败与熔断）在 `postMany` 上原样通过。
- 测试不打真网，Jev 只经 `jsonFetch` 替身。
- `pnpm check` 全绿。
