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

摘要是这条 tool_use 输入的紧凑单行，最长 80 个字符，更长就截断并加上 `…`。`<n>k` 是该块原文 token 数除以 1000，保留一位小数。打分器把块判为 `summary` 时，模型看到的是这一行桩，换行，然后 `summary (model notes, not evidence): ` 和打分器写下的笔记。笔记不是工具当时的输出。模型要当时的原文时调用 `expand`，参数 `id` 就是桩里的 chunkId。这个工具按块 id 从 Log 取回当时的 tool_result 或文本消息，不重新执行原来的工具。

expand 率 = 清单 `budget.expands` / `budget.omitted`。`expands` 是截至本轮、路径上名字为 `expand` 的 tool_use 次数；`omitted` 是本轮档位为 none 的块数。

### Token effect

进视图的申领单按所选档位计 token，总和受预算约束。清单里每张申领单都有 tier、reason、tokens；`budget` 记下 total、used、overrun。

历史按块估算。token 影响等于淘汰之后仍保留的块的 token 合计；降到 none 的工具块不再按原文占用剩余预算。清单里该块的 `tokens` 改记桩文本本身，`originalTokens` 保留原文估算。`summary` 档同样改记桩加笔记的长度，`used` 按这个长度计。`budget` 除了 total、used、overrun，还有 omitted 和 expands。

### KV cache effect

阶段内，前缀逐字节冻结，只在尾部追加。缓存断点由视图给出，适配器只翻译：

```text
["head", {afterMessage: 冻结前缀最后一条}, {afterMessage: 最后一条}]
```

`head` 盖住 system 和工具 schema。第二个断点落在本阶段第一份清单覆盖到的历史末尾（epoch 头加上当时的 seq 上限）。第三个断点落在当前最后一条消息。新块从那个 seq 之后才进入淘汰，近期窗口也只在这些新块里数。阶段重开时若用模型打分，那一次调用是上一轮主请求的前缀延长：头和历史逐字节复用，指令只追加在尾部。

开不开新阶段由这道不等式批准：

```text
(旧视图 − 新视图) × 读价 × 预计剩余轮数  ＞  新视图 × 写价 − 旧视图 × 读价
```

口径：

- 每轮节省 = (旧 − 新) × 读价
- 重建额外成本 = 新 × 写价 − 旧 × 读价。减掉的那一项是因为重建的那一轮本来也要付旧视图的读价，不能重复计
- 回本轮数 = 额外成本 ÷ 每轮节省，向上取整
- 额外成本不大于 0 就是立即回本

读价是 `cacheRead / input`。写价是 `max(cacheWrite, input) / input`。缓存写价为 0 不是免费：新前缀第一次仍按未命中的输入价付一遍（无附加费 ≠ 免费，首次仍付未命中价）。Anthropic 的写价倍率是 1.25，DeepSeek 和 OpenAI 是 1.0。目录没有定价时退回 0.1 与 1.25。旧视图 token 用上一轮 usage 实测锚定，新视图先估算再乘以 measured/estimated。省下的比例小于 `minSavingsRatio`（默认 0.2）就不开新阶段。视图估算超过上下文窗口的 0.9 时强制重开一次；这是安全阀，不是策略。重开后仍然超限就记在 `budget.overrun`，不再为同一道安全阀重复重开。

`cacheHitRatio` 仍是实测读 token 除以 `cache.expectedReadTokens`，口径不改，代码也不把差值抹平。DeepSeek 的前缀缓存以 64 token 为一块，而且只认完全相同的前缀。人工比对估算和实测时，相差不到一块是块对齐，不是没命中。

## 路由

`contextCompiler({ route })` 只在本轮决定重开阶段时调用。策略没装，或者 `rebuild` 为 false，一次都不会进策略。

传给策略的候选默认是当前 provider 的 `listModels`。要跨 provider，调用方在策略里换成已装配 provider 的目录并集。`costTierPolicy` 只看 ladder 上的型号，目录里定不了价的跳过。梯子只有当前型号时，阶段重开不换型号。`apps/sage` 的默认就是这一档；跨型号要显式传入 ladder。

`sensitivePolicy` 看的是这次新视图的块，不是整段历史。tier 为 `none` 的块不触发规则。命中的 access key 写成 `sensitive:<key>`，对应 provider 从候选里拿掉。`withSensitiveFilter` 先裁候选，再跑里面的策略。`costTierPolicy({ pick: "cheapest-allowed" })` 在剩下的 ladder 里选 input 价最低的。

选中的型号写在 `epochStart.model`。策略 reason 追加到清单的 `epochDecision.reason`，形如 `;route:<reason>`。适配器切换和 `model_change` 由内核落盘。

## 子任务

模型调用 `spawn_subtask`，参数是 `query`，可选 `budget`、`readOnly`、`model`。也可以传 `preset` 和 `args`：预设是 `<cwd>/.sage/subtasks/<name>.md` 与 `<home>/.sage/subtasks/<name>.md`，项目文件盖住同名的用户文件。名字是文件名去掉 `.md`。frontmatter 只认 `name`、`budget`、`readOnly`、`model`（`provider/model`）；`name` 不改注册名。正文是 query 模板，`{{args.xxx}}` 用 args 替换，缺的参数换成空串。调用里写明的 query、budget、readOnly、model 盖住预设。一次派多个子任务用 `spawn_subtasks`，参数是 `tasks`。全部 `readOnly: true` 时直接并行。否则看每个任务声明的 `accesses`：同一 key 上任一方是 write 就冲突；只读任务的 accesses 当空；没声明 accesses 且不是只读，视为未知写，和任何其他任务冲突。冲突则整单拒绝，正文点名 `(taskA, taskB, key)`。默认同时最多 4 个。

子任务不复制父会话的历史。它在同一份 Log 上另开一个 sessionId，第一条 entry 是 `subtask`，记下父会话和父 leaf。子会话的头（system 和工具 schema）与父上一次请求相同：工具列表不换。差异在尾部。父请求里已经冻结的前缀原样留在前面，子任务的问题作为尾部申领单 `subtask-query` 接在后面。子会话自己的块 id 带 `own:` 前缀；父块仍用 `seq:N` / `seq:N#id`，指的是同一份原文。

只读不靠换掉工具集。`readOnly: true` 时工具 schema 不动，guard 否决所有 `isReadOnly === false` 的调用，结果是 `Denied: read-only subtask`。这样前缀的 tools 块不会因为少了一把工具而失效。

子任务是一次性的。跑完把结果写进父会话的 tool_result，不保留一条可以接着聊的子会话。正文是 `Subtask <sid> <outcome>. <最后一条 assistant 文本>`，下一行是 `Chunks: child:<sid>#seq:N, …`。父会话之后调用 `expand`，id 用 `child:<sid>#<chunkId>`，取回的是那一块当时的原文。子会话的 usage 写在这次 tool_result 的 details 里。内核把它记到父用户消息的 `details.toolResults`，不累加进父会话的 `RunSummary.usage`。`health.subtasks` 汇总这些 usage；`cacheHitRatio` 和 `expandRate` 仍只算父会话。纵向默认最多到孙会话（`maxDepth` 2），每个会话横向默认最多 8 个（`maxSubtasks`）。

## 打分器

块的档位先由四条确定性规则决定：被后来的访问盖掉、失败重试的中间输出、最近的工具块、预算从最旧开始淘汰。这四条不经过打分器。打分器只看淘汰之后仍然是 `kept` 的工具块，而且只在本轮 `decideEpoch` 判定重开阶段时调用一次。阶段内不调用，前缀不用为了打分重写。

默认是 `ruleScorer`：这些块保持 full，reason 仍是 `kept`。没装打分器时，视图和清单与只跑淘汰时逐字节相同。

`modelScorer` 用调用方注入的 adapter 打一次分。请求的 `head`（system 和 tools）以及此前的 messages 与上一轮主请求逐字节相同，只在末尾多一条 user 消息：固定指令、当前问题、以及候选清单 `[{id, tool, summary, tokens}]`。用不到的 tools 也照带。少一个 tool 或换掉 system，缓存前缀会从第一个 token 断开，辅助调用就变成把整段历史再付一遍。候选超过 60 个时丢掉最旧的。`timeoutMs` 默认 20000。

模型应只返回 JSON：`{"decisions":[{"id","tier":"none"|"summary"|"full","summary"?}]}`，summary 最长 200 字符。`summary` 档在视图里是桩行，换行，然后 `summary (model notes, not evidence): ` 加上那段笔记。`none` 仍是原来的桩。`full` 保持原文。返回里没有的块保持 `kept`。打分器若把 `recent` 块判成 none，以 recent 为准。

非法 JSON、超时、adapter 的 finish error，或者还没有上一轮请求，全部退回规则打分，本轮照常发出。清单 `epochDecision.reason` 追加 `;scorer:fallback:<原因>`。这次调用的 usage 写在清单字段 `scorer: { calls, usage, fallback }` 里。

CLI `--scorer model` 给根编译器装上模型打分，沿用已装配的 adapter 和当前模型。不写这个参数就是规则打分。

`jevScorer` 调用 TypeSafe System One（模型 `jev-latest`）。它不读上一轮请求，请求里也不带历史。`POST <baseUrl>/v1/systemone`（默认 `https://api.typesafe.ai`），header 为 `Authorization: Bearer <key>` 和 `Content-Type: application/json`。`state` 是 `{ query, chunks: [{ id, tool, summary, tokens }] }`，`summary` 是那一行桩摘要，不是原文。`questions` 里每块一个 `choice`，标准是 `full`（需要原文）和 `none`（一行摘要够用）。`probabilities.full` 不低于 `minConfidence`（默认 0.5）判 full，否则 none。清单上的 reason 是 `scorer:jev:<p>`，p 为 full 的概率，保留两位。不写 summary。回答里没有的块保持 `kept`。

每批最多 `batchSize` 块（默认 40）。state 字符数按 `query.length + Σ(tokens × 4)` 估算，不超过 24000；单块超了也单独发出。没有 key、非 2xx、超时（默认 10000ms）或正文不是 answers 对象时，返回 `{ decisions: [], fallback }`，不抛。429 和 529 最多再试 3 次，间隔 `500ms × 2^n`。各批 `usage.input_tokens` 相加，写入返回 usage 的 `inputOther`，其余三项为 0。

CLI `--scorer jev` 读取 `TYPESAFE_API_KEY`。这个变量缺失时进程以退出码 2 结束，并在 stderr 点名它。目录里的 provider `typesafe` 只有 `jev-latest` 的定价（输入 $0.042/MTok，输出和缓存读写都是 0），`listModels("typesafe")` 可见。`--provider typesafe` 会报错：它不是聊天端点。

## 离线回放

离线重编译拿不到模型对新视图的反应，所以这一路不打模型。`policy.scorer.name` 不是 `rule` 时，`replayOffline` 换成 `ruleScorer`，并在返回数组上挂一个不可枚举的 `scorerNote`。`compareOffline` 读到它，就在 Markdown 和 JSON 里写上 `scorer:offline-rule`。

```ts
replayOffline(store, sessionId, policy, { tools, system })
compareOffline(diffsA, diffsB) // { markdown, json }
```

`policy` 是 `{ budget?, keepRecent?, scorer?, expectedTurns?, passthrough? }`。每一轮只用该轮视图提交之前的 path 前缀。编译器策略会把上一轮自己写下的清单放进影子 Log，后面的轮次冻结的是这套策略，不是录制时的清单。

`recordedTokens` 是该轮 assistant `details.usage` 的输入合计（`inputOther + inputCacheRead + inputCacheCreation`）。`passthrough: true` 时 `policyTokens` 等于这个数，`saved` 为 0，已有回放 case 的 `prefixStable` 全为 true。编译器策略的 `policyTokens` 是该轮 `budget.used × calibration.ratio`，路径上没有校准则倍率是 1。Log 不存上下文窗口和定价，回放用 8000 和 `pricing: null`，与无 key 回放相同。

`prefixStable` 在第一轮为 true。之后，上一轮重编译出的 head 与本轮相同，且上一轮的 messages 是本轮 messages 的 JSON 前缀，才为 true。

`omittedIds` 是本轮清单里 tier 为 `none` 的块 id。`likelyNeededOmissions` 是其中的工具块：块 id 里 `#` 后面的 `toolUseId`，或该工具 `accesses(input)` 的 `key`，出现在更后面某条 assistant 的文本或 tool_use 输入里。这是「后来被引用」的代理。

`compareOffline` 的表是每轮两边的 `policyTokens`、总 `saved`、前缀稳定轮数、疑似误删条数。Markdown 和 JSON 只有数字、id 和 `scorer:offline-rule`，没有块原文。

下一步动作是否一致、任务成不成，由真机基准比较两套策略。见仓库 [`bench/README.md`](../../bench/README.md)。基准不进 CI。

## 第一方申领单

- 系统提醒：模型在最后一条 user 消息末尾看到一行提醒；落点 `tail`；只有 line 档。
- AGENTS.md：模型在阶段头看到工作区 `AGENTS.md` 的全文，预算不够时降到前 40 行加 `…`；落点 `epoch-head`；档位 full / summary。工具 path 碰到的子目录 `AGENTS.md` 挂在尾部，正文没变就不重写全文；落点 `tail-if-changed`。
- 工具目录：模型在阶段头看到每个延迟工具一行「名字 — 说明」，full 档在这组行之后附上各自的 inputSchema；落点 `epoch-head`；档位 line / full。目录为空则不出现，也不改工具头。
- 进度账本：模型在尾部看到 Goal / Plan / Decisions，钉住不降；落点 `tail-if-changed`；只有 full 档。没有 `update_ledger` 调用则不出现。
