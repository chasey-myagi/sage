# Agent Note: jev 打分器 v2——发送原文摘录，Noul 并行判断，代码合成档位

## Problem

`jevScorer` 只把工具参数的一行摘要发给 Jev，工具输出原文完全不发。每块只问一个 `choice{full, none}`，阈值写死 0.5，没有 summary 档。分批时按 `tokens × 4` 估算字符数，但请求里并没有这些内容，估算和实际发送对不上。Jev 只能看着工具名和参数去猜以后还要不要原文。把「要不要原文」整个交给一个意图类问题，正是 Jev 文档列出的弱项：字面理解、间接推理、在大而无关的 state 里判断。

## Decision

- **每块一个请求**，同时最多 4 个在途（`MAX_PARALLEL_REQUESTS`；API 上限是每秒 10 万 token、80 个请求）。state 是 `{ task, ledger, recent_assistant, chunk }`，`chunk` 带 `id / tool / input / tokens / head / tail / omitted_lines / candidates`。state 小，Jev 只需看与这一块有关的东西。工具已标 `isError` 的块不发问，直接判 full（`scorer:jev:full:is_error`），所以 state 里也不再带 `is_error`。
- **请求体积**：候选最多 24 行，超出时保留前 8 和后 16。候选已经原样覆盖全部输出时（grep、ls 这类小输出常见），`head` 与 `tail` 置空。每个请求至多 28 题。`jev-size.test.ts` 钉住四类典型块（200 条 grep 命中、300 行 TS、150 项 ls、vitest 失败输出）的请求都不超过 28 题、20KB。
- **原文来源**：只从 `ScoreInput.prevRequest.messages` 取，`ScoreInput` 不改。入口打分和影子打分传的是本轮编好的视图，候选块此时仍是 full，原文都在。重建打分传的是从 Log 重建的上一轮请求，上一阶段被判成 none 或 summary 的块在那里只剩桩行。判断方法是与该块自己的 `renderStub(chunk)` 比前缀：桩行里有块 id 和大小，只有同一份 Log 事实渲染出的桩能对上。拿不到原文的块不发请求、不给判定，编译器原样保留。没有 `prevRequest` 时整体退回（`no-previous-request`，`requested: false`），与 `modelScorer` 一致。
- **ledger** 读 `prevRequest` 里最新的 `<claim id="progress-ledger">` 正文，用编译器的 `parseClaimMessage` 解析，不 import 进度账本插件。**recent_assistant** 是发出这次 tool_use 的那条 assistant 消息里的文字。
- **候选行由代码挑**，按工具名字符串分派（`read` / `grep` / `ls` / `find` / `bash`，其余走通用规则），不 import 工具包。测试汇总只认 `Tests` / `Test Files` 开头的行和 `<n> passed` / `<n> failed`，不收 vitest 每个文件一行的 `✓ a.test.ts (3 tests)`。
- **问题全是 Noul，一题一个条件**：`reports_failure`、`is_edit_target`（问 `chunk.input` 里那个路径的文件）、`answers_lookup`、`is_broad_listing`（只问是不是列表或宽泛搜索；是否点名了某个条目由 `answers_lookup` 管），以及每个候选一个 `keep_<i>`。criteria 写明 true 与 false 的边界，例如「读到的源码里出现 error 字样不算失败」。`keep_<i>` 是陈述句，不用反事实的两跳问法：instructions 是 `{ line, statement }`，`statement` 与 `criteria.true` 是同一句话，criteria 每条不超过 80 字符，样板约 270 字节。那一行直接写进 `line`，Jev 不必按下标去 state 里找（文档里的间接寻址弱项）。不问意图，不让 Jev 计数或算术。
- **档位由代码合成**（`decideTier`）：失败、编辑目标或查找命中不低于 `full` 判 full（新结果里正好有 assistant 要找的东西，就不摘成 3 行）；宽泛列表不低于 `none`、另外三项都低于 `low` 判 none；其余从 `keep` 达标的行里取概率最高的至多 3 行，按原文顺序用 `; ` 连接，再过 `isLocatableSummary`，过不了判 full。`isLocatableSummary` 从 `score-entry.ts` 下沉到 `score-common.ts`，入口打分与 jev 共用一份。
- **阈值默认** `{ full: 0.4, none: 0.8, low: 0.2, keep: 0.5 }`，经 `thresholds` 选项逐项覆盖。依据是两种错误的代价不对称：丢掉以后要用的证据，代价是多一次读取或走错一步；多留原文只多花 token。所以留的门槛低于均势，丢的门槛要求 Jev 很确定，并且其余判断都是明确的否。这些值没有在数据上调过，是需要实测校准的起点。
- **reason** 是 `scorer:jev:<tier>:failure=…,edit=…,lookup=…,broad=…[,keep=<最高>]`，只供审计。入口打分原来会用 `scorer:entry:<tier>` 盖掉打分器的 reason，现在 `entryReason` 把打分器的非空 reason 接在后面（`scorer:entry:none;scorer:jev:none:…`）。不带 reason 的打分器得到的字节与以前相同。
- **长度**：state 加最长问题 ≤ 32k token，state 加全部问题 ≤ 64k token。估算按每 2.5 个 UTF-8 字节算一个 token。超限先把 head 和 tail 一起减半，减到都为空仍超，再把候选减半，head 和 tail 回到默认重新试。
- **用量**：`input_tokens` 进 `usage.inputOther`，`output_tokens` 进 `usage.output`。任何一个请求失败，整次退回且没有判定；已完成请求的 usage 仍然返回，入口打分会把它记到清单上。
- **重试**：429 与 529 读 `retry-after`（秒数或 HTTP 日期），以它为准等待，最长 30 秒（`MAX_RETRY_AFTER_MS`），免得一次打分卡住整轮；没有这个头时按 `500 × 2^n` 毫秒退避。
- **截断只有一套**：能正确处理代理对的 `clipEnd` / `clipStart` 放在 `compiler/clip.ts`，`summarizeInput` 改用它，桩行里不会再切出半个 emoji。
- 删掉 `batchSize` 与 `minConfidence` 选项（预发布不留兼容层）。
- 文件拆成 `scorer/jev.ts`（工厂与并发）和 `scorer/jev/` 下的 `http` / `sources` / `candidates` / `questions` / `state` / `tiers` / `text`，每个文件只做一件事。
- 新增测试替身 `jsonFetch`，放在 `@sage/core/testing`：记录每次调用的 URL、header 和解析后的 body，按回调应答。`apps/sage/test/jev-endpoint.test.ts` 原来的手写记录器改用它。

## Alternatives considered

- **重建模式多块一批、state 只放 input 和候选行**（issue 允许的做法）：没有采用。分批能把请求数降几倍，但 Jev 的准确率随 state 里无关内容增多而下降，多块放一起正好把别的块变成干扰；分批还要第二种 state 形状、第二套问题措辞（`chunks[k]` 寻址）和按批切分的代码。逐块发只有一种形状，state 也更小。
- **给 `ScoreInput` 加模式字段，让 jev 区分入口与重建**：逐块发以后两种模式走同一条路径，不需要这个字段。
- **对没有原文的块只凭参数提问**：Jev 看不到输出，会凭参数猜失败与列表，判 none 等于盲删证据。不发、保持原档更稳。
- **沿用上一阶段的桩当作上次的判定**：需要从渲染文本里解析出档位和笔记，等于把状态编码进字符串；上一份清单的结构化行打分器又拿不到。不选。
- **候选行写在 state 里、问题用下标引用**：Jev 文档把间接寻址列为弱项。候选行仍放在 state 里供其他问题参考，`keep_<i>` 另在 instructions 里引出原行，多花几百个 token。
- **一个 `choice{full, summary, none}`**：把三种不同的判断塞进一个意图类问题，也就是 v1 的问题所在。拆成可以独立验证的 Noul，由代码合成。
- **按 4 字符一个 token 估算**：代码、哈希、CJK 的实际比例更低，会低估、超出上限。2.5 字节偏保守，代价只是在少数情况下多减几行。

## Consequences

- 每次打分的请求数等于有原文的候选块数，calls 仍记 1。阶段重开时 kept 块多，请求就多；4 路并发、10 秒单请求超时和最长 30 秒的 retry-after 等待一起决定最坏延迟，重开那一轮会比 v1 慢。
- jev 生成的 summary 是原文摘行，主模型看到的是 `summary (model notes, not evidence): <行>; <行>`，仍然标着不是证据。README 的 Model Experience 写明了这点。
- 入口打分的 reason 多了一个后缀，Log 里 `scorer:entry:*` 的根不变，`reasonKind` 的判定不受影响。
- 阈值没有经过实测，换数据集或换 Jev 版本时要重新评估。`thresholds` 选项留了口子，CLI 暂不暴露。
- 进度账本的 claim id 在 jev 里以常量出现一次，与账本插件的字面量重复。插件之间不许 import，claim 文本是两者之间唯一的约定；`jev-calls.test.ts` 经 `contextCompiler` 装上 `progressLedger`，断言发出去的 `state.ledger` 等于渲染后的账本，任何一边改名都会红。
- 桩行摘要在截断点正好落在代理对中间时，字节与以前不同（以前留下半个 emoji）。重建用同一个函数重新渲染，前后一致。
- 单个请求失败时丢弃整次结果的行为不变，另开 issue 跟进。

## Testing

- `packages/plugins/test/scorer/jev.test.ts`：请求形状（每块一个请求、state 逐字段、四个固定问题与 `keep_<i>` 的英文原文）、三档各自的分支、summary 的选行与顺序、不可定位时退回 full、阈值覆盖、没有 `prevRequest` 时退回、缺原文和桩行的块不发。
- `packages/plugins/test/scorer/jev-excerpt.test.ts`：各工具的候选行（含不收 vitest 的 `✓` 行）、24 行上限的前 8 后 16、候选覆盖全文时不发 head/tail、head/tail/omitted、32k 截断（多字节输出，断言 state 加最长问题与全部问题都在保守估算之内）、单行与参数的长度上限。
- `packages/plugins/test/scorer/jev-calls.test.ts`：429/529 重试与退避、中止、超时、缺 key、非法 JSON、500、单块失败时整体退回但保留已花的 usage、input 与 output 两项 usage；经 `contextCompiler` 的重建打分（从 Log 重建的上一轮请求取原文）、入口打分 reason 带概率、缺 key 时 0 次调用、影子调用计数。
- 把 `jev.ts` 换回 v1 实现跑这三个文件：27 个用例红。换回 v2 全绿。
- `packages/plugins/test/scorer/jev-size.test.ts`：四类典型块每个请求不超过 28 题、20KB。
- `packages/plugins/test/compiler/stub.test.ts`：截断点落在 emoji 中间时不留半个代理对。
- `pnpm check` 全绿。
