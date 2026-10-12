# @sage/plugins

插件提交申领单，编译器在总预算下分配档位，再组装成这一轮的视图。

## Model Experience

### What the model sees

模型看到的是一条阶段头 user 消息，加上追加在最后一条 user 消息末尾的 text block。阶段头里每张 epoch-head 申领单是一段，段与段之间空一行。尾部 `tail` 与 `tail-if-changed` 用同一格式，各占一个 text block，不另开消息：

```text
<claim id="…" tier="…">
正文
</claim>
```

`none` 档的申领单不进视图。被淘汰的工具块仍留下 tool_use，配对的 tool_result 正文换成一行桩，isError 不变：

```text
[#<chunkId> · <toolName> · <摘要> · <n>k tokens · omitted · expand("<chunkId>") to retrieve]
```

摘要是这条 tool_use 输入的紧凑单行，最长 80 个字符，更长就截断并加上 `…`。`<n>k` 是该块原文 token 数除以 1000，保留一位小数。打分器把块判为 `summary` 时，模型看到的是这一行桩，换行，然后 `summary (model notes, not evidence): ` 和打分器写下的笔记。笔记不是工具当时的完整输出：模型打分器的笔记是它自己写的；jev 打分器的笔记是从原文里摘出的至多 3 行，用 `; ` 连接，每行可能已被截短。模型要当时的原文时调用 `expand`，参数 `id` 就是桩里的 chunkId。这个工具按块 id 从 Log 取回当时的 tool_result 或文本消息，不重新执行原来的工具。

expand 率 = 清单 `budget.expands` / `budget.omitted`。`expands` 是截至本轮、路径上名字为 `expand` 的 tool_use 次数；`omitted` 是本轮档位为 none 的块数。

### Token effect

进视图的申领单按所选档位计 token，总和受预算约束。清单里每张申领单都有 tier、reason、tokens；`budget` 记下 total、used、overrun。

历史按块估算。token 影响等于淘汰之后仍保留的块的 token 合计；降到 none 的工具块不再按原文占用剩余预算。清单里该块的 `tokens` 改记桩文本本身，`originalTokens` 保留原文估算。`summary` 档同样改记桩加笔记的长度，`used` 按这个长度计。`budget` 除了 total、used、overrun，还有 omitted、expands、frozenTokens 和 staleOmittable。`frozenTokens` 是本轮冻结视图里档位不是 none 的已冻结块的 tokens 合计。`staleOmittable` 是其中若现在做一次不带冻结的全量重编译会变成 none 的那部分。重开当轮这两个数仍按重开前那份冻结视图计算。阶段第一轮没有冻结记录，两者都是 0。子任务重编译不比较重开，这两个数是 0。

### KV cache effect

阶段内，前缀逐字节冻结，只在尾部追加。缓存断点由视图给出，适配器只翻译：

```text
["head", {afterMessage: 冻结前缀最后一条}, {afterMessage: 最后一条}]
```

`head` 盖住 system 和工具 schema。第二个断点落在本阶段第一份清单覆盖到的历史末尾（epoch 头加上当时的 seq 上限）。第三个断点落在当前最后一条消息。阶段内，一个块的档位以它第一次出现在本阶段清单里的那次为准，后面的清单不改它。`superseded` 和预算降档都不作用于已冻结的块；它们可以过时，但不能变。淘汰只决定还没冻结的新块，输入里带上已冻结块作只读参考：新块在历史末尾，不会被更早的块盖掉，所以第一次进清单时是 `recent` 或 `full`。预算先扣掉已冻结且仍保留的 token，只把新块从最旧开始降到 none，降完的档位同样冻结。缓存断点仍停在阶段第一份清单的 seq 上限，新内容写在那个断点之后。阶段重开时若用模型打分，那一次调用是上一轮主请求的前缀延长：头和历史逐字节复用，指令只追加在尾部。请求带 `toolChoice: "none"`，不删 tools，前缀缓存仍能命中。

#### 重开不等式

开不开新阶段由这道不等式批准：

```text
(旧视图 − 新视图) × 读价 × 预计剩余轮数  ＞  新视图 × 写价 − 旧视图 × 读价 + 预计取回 × 写价
```

口径：

- 每轮节省 = (旧 − 新) × 读价
- 重建额外成本 = 新 × 写价 − 旧 × 读价 + 重建后代价。减掉的那一项是因为重建的那一轮本来也要付旧视图的读价，不能重复计
- 回本轮数 = 额外成本 ÷ 每轮节省，向上取整，记在 `epochDecision.paybackTurns`
- 额外成本不大于 0 就是立即回本（`immediate`），不受下面的承诺限制：那一轮本身就更便宜，挡住它等于为沉没成本买单
- 剩余轮数 = max(1, `expectedTurns` − 已开始轮数, ceil(`alpha` × 已开始轮数))。已开始轮数是这一轮的 turn 减 1。`expectedTurns` 默认 20，`alpha` 默认 0.5，都是 `contextCompiler` 的选项。清单 `epochDecision.remainingTurns` 记下这一轮实际用的数
- 省下的比例小于 `minSavingsRatio`（默认 0.2）就不开新阶段

前十几轮里，`expectedTurns` 减去已开始轮数仍然更大，视界和固定 20 相同。再往后，`ceil(alpha × 已开始轮数)` 抬过那条直线：跑得越久，假定后面还有大约一半那么多轮。固定 20 会从 turn 20 起把剩余夹成 1，回本要几十轮的长任务就永远开不了新阶段。不设上限会让任何正节省都立刻重开，冻结前缀每轮作废。这个下界只改「还打算走多少轮」，不改上面的不等式。

#### 阶段承诺的回本

按回本批准的重开，等于承诺这个新阶段至少活过它自己的回本轮数，否则那次写永远赚不回来。所以阶段的第一份清单（就是批准重开的那一份）里的 `paybackTurns` 是这个阶段的承诺，下一轮起记在 `epochDecision.committedPaybackTurns`。阶段从第一份清单的 turn 算起已过的轮数记在 `turnsSinceEpochOpen`。已过轮数还没到承诺值时，按回本批准的重开改成不重开，原因 `payback-pending:<已过轮数><<承诺>`。两个数都从 Log 读出，重放得到同样的判断。阶段 0、立即回本开的阶段、安全阀开的阶段没有承诺（记 0）。上下文窗口安全阀与立即回本不受这条限制。

#### 重建后代价

重建后代价 = 预计取回 × 写价，清单记作 `epochDecision.postRebuildCost`。预计取回是这次重建甩掉的 token 里预计会被 `expand` 拿回来的部分，不超过 旧 − 新。本会话已经有成功的 `expand` 时，按「成功次数 ÷ 已有阶段数 × 平均每次结果 token」估；这是上界：阶段 0 因预算省略而触发的 `expand` 也算进去，取回的原文也全按写价计。还没有观测时按先验 `PRIOR_EXPAND_SHARE`（0.1）乘以 旧 − 新，让回本轮数多出 `0.1 × 写价 ÷ 读价` 轮，和视图大小无关。来源记在 `epochDecision.expandEstimate`：`{source: "observed", expandsPerEpoch, tokensPerExpand}` 或 `{source: "prior", share}`。

#### 费率

两个倍率取自这次调用的定价，也就是本轮 `request_header.pricing` 记下的那一份。读价是 `cacheRead / input`。写价是 `max(cacheWrite, input) / input`：写价低于输入价时按输入价算，新前缀第一次总要按未命中价付一遍。内置目录里价格页写明「未命中按输入价、无创建费」的型号（OpenAI、DeepSeek、Kimi 自动缓存、百炼隐式缓存；sage 不发 `cache_control`，百炼走的是隐式缓存），`cacheWritePerMTok` 就写成输入价，写价倍率 1.0；Anthropic 是 1.25。只有定价缺失（或输入价为 0）时才用默认值 0.1 与 1.25，清单 `epochDecision.rateSource` 记 `default`，原因追加 `;pricing:fallback`；否则记 `pricing`。

#### 估算与校准

清单 `budget.used` 只含申领单和块，不含系统提示、工具 schema 和任务消息这截固定开销。校准记在 `calibration`：`measured` 是上一轮 assistant usage 的输入合计，`estimated` 是那一轮的 `budget.used`。`overheadTokens` 取本阶段第一轮（没有 usage 时取路径上最近一次算得出的轮）的 `measured − budget.used`，小于 0 记 0：负的固定开销会把新视图 token 打成负数，成本式会把它当成立即回本，再开一阶段。之后各轮 `ratio = (measured − overheadTokens) / estimated`，记在清单上，给离线 `policyTokens` 用。缺实测或 `estimated` 为 0 时不写校准，调用方用 `overheadTokens = 0`、`ratio = 1`。

旧视图 token 是 `overheadTokens + 冻结视图 used × ratio`。新视图 token 是 `overheadTokens + 全量重编译 used × ratio`。锚点那一轮的 ratio 正好是 1。ratio 大于 1 也照乘，否则估算系统性低于实测，安全阀会漏报。ratio 小于 1 时同样乘回去，把计数器偏高的部分收回来。两边都加同一份固定开销，差值仍只来自块的档位。

#### 上下文窗口安全阀

`overheadTokens + used × ratio` 超过上下文窗口的 0.9 时强制重开一次；这是安全阀，不是策略。重开后仍然超限就记在 `budget.overrun`，不再为同一道安全阀重复重开。重开那一轮的视图是全量重编译，随后按新阶段重新冻结，固定开销也从新阶段的第一轮重算。

#### 命中对账

`cache.expectedReadTokens` 是上一轮实际发出去的冻结前缀估算：用上一轮清单自己的 `calibration.overheadTokens` 与 `calibration.ratio` 去乘那一轮的 `budget.frozenTokens`，即 `overheadTokens + ratio × frozenTokens`。上一轮没有校准时固定开销为 0、ratio 为 1。重开后本轮重新锚上的固定开销不进这个数。`cache.measuredReadTokens` 是那一轮 usage 的 `inputCacheRead`。两者同口径，expected 与 measured 差距大就是前缀被砸了。`cacheHitRatio` 仍是实测读 token 除以 `cache.expectedReadTokens`，口径不改，代码也不把差值抹平。DeepSeek 的前缀缓存以 64 token 为一块，而且只认完全相同的前缀。人工比对估算和实测时，相差不到一块是块对齐，不是没命中。

## 路由

`contextCompiler({ route })` 只在本轮决定重开阶段时调用。策略没装，或者 `rebuild` 为 false，一次都不会进策略。

传给策略的候选默认是当前 provider 的 `listModels`。要跨 provider，调用方在策略里换成已装配 provider 的目录并集。`costTierPolicy` 只看 ladder 上的型号，目录里定不了价的跳过；候选条目自带的价格不算数。调用方传 `localModels`（用户文件解析出的目录条目）时，目录是内置表加上这些条目。梯子只有当前型号时，阶段重开不换型号。`apps/sage` 的默认就是这一档；跨型号要显式传入 ladder。

`sensitivePolicy` 看的是这次新视图的块，不是整段历史。tier 为 `none` 的块不触发规则。命中的 access key 写成 `sensitive:<key>`，对应 provider 从候选里拿掉。`withSensitiveFilter` 先裁候选，再跑里面的策略。`costTierPolicy({ pick: "cheapest-allowed" })` 在剩下的 ladder 里选 input 价最低的。

选中的型号写在 `epochStart.model`。策略 reason 追加到清单的 `epochDecision.reason`，形如 `;route:<reason>`。适配器切换和 `model_change` 由内核落盘。

## 子任务

模型调用 `spawn_subtask`，参数是 `query`，可选 `budget`、`readOnly`、`model`。也可以传 `preset` 和 `args`：预设是 `<cwd>/.sage/subtasks/<name>.md` 与 `<home>/.sage/subtasks/<name>.md`，项目文件盖住同名的用户文件。名字是文件名去掉 `.md`。frontmatter 只认 `name`、`budget`、`readOnly`、`model`（`provider/model`）；`name` 不改注册名。正文是 query 模板，`{{args.xxx}}` 用 args 替换，缺的参数换成空串。调用里写明的 query、budget、readOnly、model 盖住预设。一次派多个子任务用 `spawn_subtasks`，参数是 `tasks`。全部 `readOnly: true` 时直接并行。否则看每个任务声明的 `accesses`：同一 key 上任一方是 write 就冲突；只读任务的 accesses 当空；没声明 accesses 且不是只读，视为未知写，和任何其他任务冲突。冲突则整单拒绝，正文点名 `(taskA, taskB, key)`。默认同时最多 4 个。

子任务不复制父会话的历史。它在同一份 Log 上另开一个 sessionId，第一条 entry 是 `subtask`，记下父会话和父 leaf。子会话的头（system 和工具 schema）与父上一次请求相同：工具列表不换。差异在尾部。父请求里已经冻结的前缀原样留在前面，子任务的问题作为尾部申领单 `subtask-query` 接在后面。子会话自己的块 id 带 `own:` 前缀；父块仍用 `seq:N` / `seq:N#id`，指的是同一份原文。

只读不靠换掉工具集。`readOnly: true` 时工具 schema 不动，guard 否决所有 `isReadOnly === false` 的调用，结果是 `Denied: read-only subtask`。这样前缀的 tools 块不会因为少了一把工具而失效。

子任务是一次性的。跑完把结果写进父会话的 tool_result，不保留一条可以接着聊的子会话。正文是 `Subtask <sid> <outcome>. <最后一条 assistant 文本>`，下一行是 `Chunks: child:<sid>#seq:N, …`。父会话之后调用 `expand`，id 用 `child:<sid>#<chunkId>`，取回的是那一块当时的原文。两个工具都把子会话写成 tool_result details 的 `childRuns`，每个子会话一项，带 `sessionId`、`outcome`、`usage`、`chunks`。内核把它记到父用户消息的 `details.toolResults`，不累加进父会话的 `RunSummary.usage`。`health.subtasks` 汇总这些 usage；`cacheHitRatio` 和 `expandRate` 仍只算父会话。纵向默认最多到孙会话（`maxDepth` 2），每个会话横向默认最多 8 个（`maxSubtasks`）。

## 打分器

块的档位先由四条确定性规则决定：被后来的访问盖掉、失败重试的中间输出、最近的工具块、预算从最旧开始淘汰。这四条不经过打分器。打分器只看淘汰之后仍然是 `kept` 的工具块，而且只在本轮 `decideEpoch` 判定重开阶段时调用一次。阶段内不调用，前缀不用为了打分重写。

默认是 `ruleScorer`：这些块保持 full，reason 仍是 `kept`。没装打分器时，视图和清单与只跑淘汰时逐字节相同。

`modelScorer` 用调用方注入的 adapter 打一次分。请求的 `head`（system 和 tools）以及此前的 messages 与上一轮主请求逐字节相同，`toolChoice` 为 `"none"`，只在末尾多一条 user 消息：固定指令、当前问题、以及候选清单 `[{id, tool, summary, tokens}]`。用不到的 tools 也照带。少一个 tool 或换掉 system，缓存前缀会从第一个 token 断开，辅助调用就变成把整段历史再付一遍。`tool_choice` 不改这些字节。候选超过 60 个时丢掉最旧的。`timeoutMs` 默认 20000。

指令说明这是旁路判定，不是继续任务。每个候选是早先某次工具调用的结果。模型只返回 JSON 数组，每项 `{"id","tier":"none"|"summary"|"full","summary"?}`，summary 最长 200 字符。摘要必须让后面的轮次不用重读就能定位：文件读取写路径、关键符号或导出、以及行号范围；grep 或 ls 写命中路径与行号；命令输出写命令和结论性数字或状态，例如 `tests 23 passed 1 failed: name`。不要只写判断（已看过、不再需要、和之前一样），那种情况改判 `none` 或 `full`。示例里的 summary 是路径、符号和行号。正文可以包在一层 json 代码围栏里。围栏去掉之后，从第一个 `[` 或 `{` 起按括号配对取出一个 JSON 值，字符串内部的括号不计入配对，这个值前后的文字都忽略。取不到值，或取出的文本不是 JSON，整次解析失败。这样解析成功时，即使原文前后还有解释，也不算 fallback，不写 `raw`。顶层如果是带 `id` 的单个对象，会包成一项。`id` 不在这份候选清单里的项丢掉并计数，其余照用。空数组 `[]` 表示全部保留，不是失败。某一项缺 `id` 或 tier 非法，整次解析失败。`summary` 档在视图里是桩行，换行，然后 `summary (model notes, not evidence): ` 加上那段笔记。`none` 仍是原来的桩。`full` 保持原文。返回里没有的块保持 `kept`。打分器若把 `recent` 块判成 none，以 recent 为准。

非法 JSON、超时、adapter 的 finish error，或者还没有上一轮请求，全部退回规则打分，本轮照常发出。清单 `epochDecision.reason` 追加 `;scorer:fallback:<原因>`。这次调用写在清单字段 `scorer: { calls, usage, fallback, raw? }` 里。`calls` 只数真正发出的打分请求：打分器在发请求前就放弃（没有上一轮请求、没有 adapter、jev 没有 key）时记 0，其余记 1。`raw` 只在 fallback 且已经拿到模型回复时出现：`text` 是原文前 300 字，`finish` 是结束种类。成功不写 `raw`。

CLI `--scorer model` 给根编译器装上模型打分，沿用已装配的 adapter 和当前模型。不写这个参数就是规则打分。

`jevScorer` 调用 TypeSafe System One，每个候选块单独一个请求，同时最多 4 个在途（API 的上限是每个 key 每秒 10 万 token、80 个请求）。工具已标 `isError` 的块不发请求，直接判 full，reason 为 `scorer:jev:full:is_error`。`model` 缺省 `jev-latest`；传入 `model` 就改用那个名字，用来对准 LLM2Jev 的 `--served-model-name`。`POST <baseUrl>/v1/systemone`（默认 `https://api.typesafe.ai`），header 为 `Authorization: Bearer <key>` 和 `Content-Type: application/json`。

`state` 是 `{ task, ledger, recent_assistant, chunk }`。`task` 是当前问题，最多 2000 字。`ledger` 是 `prevRequest` 里最新一张 `progress-ledger` 申领单的正文，最多 3000 字，没有就是空串。`recent_assistant` 是发出这次 tool_use 的那条 assistant 消息里的文字，保留末尾 1500 字。`chunk` 是 `{ id, tool, input, tokens, head, tail, omitted_lines, candidates }`：`input` 是工具参数的紧凑 JSON，最多 1000 字；`head` 与 `tail` 是原文前 40 行和后 20 行，每行最多 400 字，`omitted_lines` 是中间没发的行数；`candidates` 是代码按工具名挑出的候选行，最多 24 行，每行最多 200 字，超出时保留前 8 和后 16（失败和汇总多在末尾）。候选行已经原样覆盖全部非空输出行（没有截短、合并或被上限切掉）时，`head` 与 `tail` 为空，`omitted_lines` 为 0。`read` 挑声明与导出，写成 `L<行号>: <行>`，行号从参数 `offset` 起算；`grep` 取每条命中；`ls` 与 `find` 取每个条目；`bash` 取状态行和下面这些行；其他工具只取这些行：失败或异常字样、栈帧、`路径:行号`、声明与导出、测试汇总（`Tests` / `Test Files` 开头的行，或 `<n> passed` / `<n> failed`；vitest 每个文件一行的 `✓ a.test.ts (3 tests)` 不算）、diff hunk 头。典型的块（200 条 grep 命中、300 行 TS 文件、150 项 ls、vitest 失败输出）每个请求不超过 28 题、约 20KB。插件只按工具名分派，不 import 工具包。

原文只从 `prevRequest.messages` 里该块的 tool_result 取。入口打分与影子打分传的是本轮编好的视图，候选块都有原文。重建打分传的是上一轮请求，上一阶段被判成 none 或 summary 的块在那里只剩桩行。拿不到原文（缺失，或内容就是该块自己的桩行）的块不发请求，也不给判定，编译器按原样保留。一个块都没发时返回 `{ decisions: [], requested: false }`。没有 `prevRequest` 时不发请求，返回 `fallback: "no-previous-request"`，带 `requested: false`。

问题全是 Noul，`criteria` 写明 true 与 false 的边界，只问输出里有什么，不问意图，不让 Jev 计数或算术：`reports_failure`（输出是否报告失败；读到的源码里出现 error 字样不算）、`is_edit_target`（`chunk.input` 里那个路径的文件是否是 `recent_assistant` 或 `ledger` 说要改的文件）、`answers_lookup`（输出里是否有 `recent_assistant` 正在找的符号、行或值）、`is_broad_listing`（是不是目录列表或宽泛的搜索结果；是否点名了某个条目交给 `answers_lookup`），以及每个候选行一个 `keep_<i>`。`keep_<i>` 是陈述句，instructions 是 `{ line, statement }`，把那一行直接写进去，Jev 不必按下标去 state 里找；`statement` 与 `criteria.true` 是同一句「`line` 指出了 `task` 或 `recent_assistant` 需要的位置或结果」，criteria 每条不超过 80 字符。

档位由代码合成，阈值取 `thresholds` 选项，缺省 `DEFAULT_JEV_THRESHOLDS`：`reports_failure`、`is_edit_target` 或 `answers_lookup` 不低于 `full`（0.4）判 full（新结果里正好有 assistant 要找的东西，就不摘成 3 行）；`is_broad_listing` 不低于 `none`（0.8），且另外三项都低于 `low`（0.2）判 none；其余判 summary，从 `keep_*` 不低于 `keep`（0.5）的行里取概率最高的至多 3 行，按原文顺序用 `; ` 连接，截到 200 字，仍须通过入口打分用的可定位检查，过不了就判 full。丢掉以后还要用的证据，代价是多一次读取或走错一步；多留原文只多花 token，所以留的门槛低、丢的门槛高。清单上的 reason 是 `scorer:jev:<tier>:failure=<p>,edit=<p>,lookup=<p>,broad=<p>`，有候选时再加 `,keep=<最高 p>`，保留两位，只供审计，代码不读。四个固定回答缺任何一个，这块不给判定。

长度遵守 Jev 的两条限制：state 加最长的一个问题不超过 32k token，state 加全部问题不超过 64k token。估算按每 2.5 个 UTF-8 字节算一个 token，比真实分词偏多。超了先把 head 和 tail 一起减半，减到都为空仍超，再把候选减半，head 和 tail 从默认重新开始；什么都减完仍超，这块不发。

没有 key、非 2xx、超时（默认 10000ms）、调用方中止或正文不是 answers 对象时，返回 `{ decisions: [], fallback }`，不抛。任何一个块的请求失败，整次结果都没有判定；第一次失败之后不再发新请求，已在途的照常结束，已完成请求的 usage 照记。没有 key 时不发请求，结果带 `requested: false`，清单上的 `calls` 记 0；发了请求时 `calls` 记 1，不论发了几个。429 和 529 最多再试 3 次。响应带 `retry-after`（秒数或 HTTP 日期）时按它等待，最长 30 秒；没有时间隔 `500ms × 2^n`。各请求的 `usage.input_tokens` 相加写入 `inputOther`，`usage.output_tokens` 相加写入 `output`；Jev 的输出不计费，记下来只为对账。

CLI `--scorer jev` 和 `--shadow-scorer jev` 读 `JEV_BASE_URL`（缺省 `https://api.typesafe.ai`）、`JEV_API_KEY`（缺省回退 `TYPESAFE_API_KEY`）和 `JEV_MODEL`（缺省 `jev-latest`）。两个 key 都没有时进程以退出码 2 结束，stderr 点名这两个变量。接到本地 LLM2Jev 时，例如 `JEV_BASE_URL=http://127.0.0.1:30000 JEV_API_KEY=local JEV_MODEL=<served-model-name>`。目录里的 provider `typesafe` 只有 `jev-latest` 的定价（输入 $0.042/MTok，输出和缓存读写都是 0），`listModels("typesafe")` 可见。`--provider typesafe` 会报错：它不是聊天端点。

## 影子打分

`contextCompiler({ shadowScorer })` 在这一轮的档位、冻结和预算都已经定下来之后，再对**本轮新进入视图的工具块**调用 `shadowScorer.score`。新块是上一份清单的 `chunks` 里没有的 chunk id。第一轮没有上一份清单，全部工具块都算新的。散文块不打。`prevRequest` 是这一轮已经编译好的视图（head、messages、缓存断点），不是上一轮发出去的请求。

结果只写 `manifest.shadow`：`{ scorer, calls, decisions: [{ id, tier, reason? }], usage?, fallback?, raw? }`。`calls` 与重建打分、入场打分同一口径，只数真正发出的请求：打分器在发请求前就放弃（`requested: false`，例如 jev 缺 key）或请求没能构造出来时为 0，其余为 1，打分器抛错也算 1。`raw` 与上面的打分记录一样，只在 fallback 时有。影子只记不生效。它不改任何档次，不改冻结，不改预算，也不改模型看到的消息。打分器抛错时 `shadow.fallback` 记下错误文本，`decisions` 为空，这一轮照常返回。打分器自己返回的 `fallback` 同样只记在这个字段上。没有新的工具块就不调用，清单上也不写 `shadow`。

子任务重编译不跑影子打分。任务结束后用 `sage stats` 对账：被影子判为 `none` 的块，后来有没有被 `expand` 取回，或者作为 `read` 结果在后续轮次被同一路径再次读取。口径见 [`apps/sage/README.md`](../../apps/sage/README.md)。

## 入口打分

`contextCompiler({ scorerApply: "rebuild" | "entry", minScoreTokens })`。默认 `"rebuild"`，也就是上面那条：只在阶段重开时给仍然 `kept` 的工具块打分。`minScoreTokens` 默认 300。

`"entry"` 改在工具结果**进入视图的那一轮**定档。本轮新出现、当前仍是 full、token 不少于 `minScoreTokens` 的工具块，打分器调用一次。`expand` 的结果不进候选，否则刚取回的原文会在同一轮被桩掉。`none` 写成可 expand 的桩。`summary` 用打分器给的笔记，最长 200 字；没给笔记就保持 full。写了笔记但不足 20 字（含空串），或不含路径、符号或数字（斜杠、扩展名、数字、驼峰或下划线标识符）时，该块按 full 处理，reason 为 `scorer:entry:full;summary-rejected`。`full` 原文不动。清单上这块的 reason 是 `scorer:entry:<tier>`；打分器给了自己的 reason 时（例如 jev 的各项概率），接在后面作为 `;<该 reason>`。打分抛错、返回 `fallback`、或回答里没有这个 id，这块保持 full，编译照常。`calls` 与重建打分同一口径，只数真正发出的请求：打分器在发请求前就放弃（`requested: false`，例如没有 adapter）记 0，其余情况包括抛错都记 1。没有候选就不调用，清单上也不写 `entryScored`。

判定写进当轮清单，下一轮起由累积冻结保住，不再被 `keepRecent` 抬回 full。已打分的块也不再占最近窗口的名额。`entry` 不再跑重建路径上的那次打分，也不再跑影子：同一次调用的结果既写清单又改视图。`scorerApply` 仍是 `"rebuild"` 时，重建打分和影子都跟以前一样。

选入口而不是「滑出最近窗口时再改档」：DeepSeek 的缓存是前缀缓存。出窗口时改档，会让该位置之后的近期内容每轮重新全价编码。按写入价和缓存读价的差距，要很多轮才回本。入口定档则这块从第一次出现就是最终形态，前缀逐字节不动。代价是模型偶尔需要全文时多一次 `expand`。

`manifest.entryScored` 是 `{ calls, none, summary, full, summaryRejected, usage? }`。`summaryRejected` 是退回的笔记数，这些块同时计入 `full`。`runHealth` 把各轮加总，没写 `summaryRejected` 的清单这一项加 0。usage 四项与 `shadow.usage` 同一口径。

## 离线回放

`replayOffline` / `compareOffline` 在 `src/replay/offline.ts`，不从包入口导出，只有本包的回放测试在用。

离线重编译拿不到模型对新视图的反应，所以这一路不打模型。`policy.scorer` 没有 `isNoop`（只有 `ruleScorer` 设了它）时，`replayOffline` 换成 `ruleScorer`，并记住这次返回的数组。`compareOffline` 收到这个数组，就在 Markdown 和 JSON 里写上 `scorer:offline-rule`；数组的拷贝不带这个标记。

```ts
replayOffline(store, sessionId, policy, { tools, system })
compareOffline(diffsA, diffsB) // { markdown, json }
```

`policy` 是 `{ budget?, keepRecent?, scorer?, expectedTurns?, alpha?, passthrough? }`。`alpha` 缺省为 `DEFAULT_ALPHA`；回放自适应视界之前录下的会话时传 `alpha: 0`，剩余轮数就退回 `max(1, expectedTurns − 已开始轮数)`。`budget` 缺省为导出常量 `DEFAULT_COMPILER_BUDGET`（40000），sage CLI 不带 `--budget` 时用的是同一个常量。每一轮只用该轮视图提交之前的 path 前缀。编译器策略会把上一轮自己写下的清单放进影子 Log，后面的轮次冻结的是这套策略，不是录制时的清单。影子 Log 用内核的 `appendRequestEntries` 写，和线上会话是同一份落盘逻辑：`request_header` 只在 system / tools 的 hash 首见或变化时带全文。

`recordedTokens` 是该轮 assistant `details.usage` 的输入合计（`inputOther + inputCacheRead + inputCacheCreation`）。`passthrough: true` 时 `policyTokens` 等于这个数，`saved` 为 0，已有回放 case 的 `prefixStable` 全为 true。编译器策略的 `policyTokens` 是该轮 `budget.used × calibration.ratio`，路径上没有校准则倍率是 1。这个乘积跟的是块和申领单；固定开销在 `calibration.overheadTokens` 里，不进 `policyTokens`。阶段决策的 old/new 是 `overheadTokens + used × ratio`，ratio 大于 1 也照乘。每一轮的上下文窗口和定价取自该轮录下的 `request_header`，不查当前目录，所以目录改价或改窗口不会改变回放的阶段决策。录下的 header 缺这两个字段时，store 校验就不通过。

`prefixStable` 在第一轮为 true。之后，上一轮重编译出的 head 与本轮相同，且上一轮的 messages 是本轮 messages 的 JSON 前缀，才为 true。

`omittedIds` 是本轮清单里 tier 为 `none` 的块 id。`likelyNeededOmissions` 是其中的工具块：块 id 里 `#` 后面的 `toolUseId`，或该工具 `accesses(input)` 的 `key`，出现在更后面某条 assistant 的文本或 tool_use 输入里。这是「后来被引用」的代理。

`compareOffline` 的表是每轮两边的 `policyTokens`、总 `saved`、前缀稳定轮数、疑似误删条数。Markdown 和 JSON 只有数字、id 和 `scorer:offline-rule`，没有块原文。

下一步动作是否一致、任务成不成，由真机基准比较两套策略。见仓库 [`bench/README.md`](../../bench/README.md)。基准不进 CI。

## 第一方申领单

- 系统提醒：模型在最后一条 user 消息末尾看到一行提醒；落点 `tail`；只有 line 档。
- AGENTS.md：模型在阶段头看到工作区 `AGENTS.md` 的全文，预算不够时降到前 40 行加 `…`；落点 `epoch-head`；档位 full / summary。工具 path 碰到的子目录 `AGENTS.md` 挂在尾部，正文没变就不重写全文；落点 `tail-if-changed`。
- 工具目录：模型在阶段头看到每个延迟工具一行「名字 — 说明」，full 档在这组行之后附上各自的 inputSchema；落点 `epoch-head`；档位 line / full。目录为空则不出现，也不改工具头。
- 进度账本：模型在尾部看到 Goal / Plan / Decisions，钉住不降；落点 `tail-if-changed`；只有 full 档。没有 `update_ledger` 调用则不出现。
