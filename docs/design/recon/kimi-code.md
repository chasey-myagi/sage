# 参考仓侦察：kimi-code

- **是什么**：Moonshot 的终端编码 agent（CLI/TUI + web + VSCode + ACP + 本地 server），MIT，monorepo；`packages/agent-core-v2` 是正在替换 v1 的新引擎（DI Scope 架构），本报告几乎只看它。
- **规模**（实测，排除 node_modules）：全仓 `*.ts/tsx` 761,989 行 / 17 个 package + 5 个 app；`agent-core-v2` 源码（非测试）90,147 行，测试 97,073 行 / 265 个测试文件。
- **主语言**：TypeScript（Node ≥ 24，pnpm，vitest，oxlint）。
- **最后活跃**：本地 checkout 停在 `main` 的 `c497af6`，2026-07-25。
- **和备忘观点的关系**：**它已经把「只追加的 Log」做成了内核事实**（每个 agent 一份 `wire.jsonl`，状态全部由纯 reducer 从 Op 流算出），也已经在**同一份 Log 上编译出两份不同规则的视图**（给模型的 / 给人的）。它没有的是备忘的后半段：没有 claim/预算/档位，没有阶段冻结与成本不等式，压缩仍是阈值触发的有损摘要，子 agent 靠**复制**日志而不是再编译一次。

路径根：`/Users/chasey/Dev/agent-developers/kimi-code`，下文 `file:line` 均相对该根。

---

## 1. 状态与历史

**机制**
- 每个 agent 一份只追加的 JSONL 日志 `wire.jsonl`（`src/wire/record.ts:14`）。写进去的不是消息，是 **Op**：`defineOp` 在 import 期把描述符注册进全局 `OP_REGISTRY`（`src/wire/op.ts:62,104`），每个 Op 带一个**纯** `apply(state, payload)` 和一份 zod `schema`；`WireService.execute` 先 apply 再 `Object.freeze`，再决定落盘与发事件（`src/wire/wireService.ts:245`）。
- 会话状态 = 若干个 `defineModel` 声明的 Model（`src/wire/model.ts:69`），恢复就是从头重放日志（`wireService.ts:136`）：未知 record 跳过并报错、schema 不过跳过、协议版本按迁移链升级、必要时原子重写整份日志（`wireService.ts:197`）。一个 Model 还能注册**跨模型 reducer**（`model.ts:67`），让别的域的 record 顺带改自己的状态，避免为一个副作用再写一条记录。
- 对话历史只是其中一个 Model：`ContextModel = ContextMessage[]`（`src/agent/contextMemory/contextOps.ts:103`），由 `context.append_message` / `context.append_loop_event` 折出来。
- **改写历史也是追加**：撤销是 `context.undo`，`apply` 里现算切点（`contextOps.ts:295,355`）；压缩是 `context.apply_compaction`（`contextOps.ts:184`），reducer 用摘要替换前缀。原始记录仍在文件里 —— 状态被压缩了，**日志没有**。
- 崩溃语义在存储层而不是内核：JSONL 解码时「尾部撕裂的最后一行直接丢弃，其它位置的损坏抛错」（`src/persistence/backends/node-fs/appendLogStore.ts:8,97`）。
- 分支 = `fork`：**物理复制** session 目录与每个 agent 的 `wire.jsonl`，再追加一条 fork 边界（`src/app/sessionLifecycle/sessionLifecycleService.ts:363,502`）。注释明说不设准入门：复制活日志得到的撕裂前缀「正是崩溃留下的状态，重放本来就会归一化」。

**对 sage 的判断**
- `抄`：**Op + 纯 reducer + import 即注册**。sage 的 Log 如果只存消息，第一次要存「压缩发生过」「模型换过」时就会退化成特判；存 Op 则天然是一条能重放的因果链，和「内核只有 Log 与 View seam」严格相容 —— 内核仍然不认识任何 Op 的语义。
- `抄`：**撤销与压缩都是追加**。这是备忘「Log 只追加、从不回头改」落到实处的唯一做法，也顺手让「第 12 轮那份原文还在不在」有确定答案。
- `抄`：**尾部撕裂即丢弃**。比 harness-pi 的 HWM 计数更省：持久化水位是为了「不重不漏」，而逐行 JSONL + 丢弃半行就已经保证了，不需要内核再持有一个计数器。
- `不抄`：**fork = 复制日志**。备忘裁决①点名的就是这笔放大（1000 条 × 8 分支）。sage 的分支应当是「同一份 Log、换个问题换个预算再编译一次」，复制只在跨机器导出时才需要。

---

## 2. 发给模型的内容怎么构造

**机制**
- 有一条明确的投影层：`AgentContextProjectorService.project(history) -> Message[]`（`src/agent/contextProjector/contextProjectorService.ts:51`），纯变换，**只读**，历史一个字不改。投影顺手做修复：错位的 tool result 挪回它的 call、丢失的 result 合成一条、孤儿/重复丢弃、开头的非 user 消息丢弃、连续 assistant 合并、空白与「整条无内容」丢弃 —— 每一处修复都产出一个 anomaly，去重后打一条告警 + `context_projection_repaired` 埋点（`contextProjectorService.ts:87,91`）。
- 同一条历史有多个投影变体：`projectStrict`、`projectMediaDegraded`（撞 413 后只保留最近 2 份媒体）、`projectMediaStripped`（图片格式被拒后按快照抹掉那一批）——都**只在读侧**，历史仍带着媒体（文件头注释写死了这一点）。
- 压缩触发是阈值：`triggerRatio 0.85` + `reservedContextSize 50_000` 两个条件任一成立（`src/agent/fullCompaction/strategy.ts:18,110`）。切点由 `canSplitAfter` 决定：user 消息之后不切、带 toolCalls 的 assistant 之后不切、下一条是 tool 时不切、前缀停在未闭合的 tool 交换时不切（`strategy.ts:236`）。窗口选择还会兜住「压缩输入本身超窗」的情况（`fitCompactCountToWindow`）。
- 压缩不是一刀切摘要：真实 user 提问**逐字保留**（超长的取头 2k + 尾，中间留一条 elision 标记），只有其余部分进摘要；摘要正文前面钉着一段固定前言，明说「这是你自己的笔记，不是证据，说做过的事请自己复核」（`src/agent/contextMemory/compactionHandoff.ts:11,12` 与 `compaction-summary-prefix.md`）。
- token 账分两段：**已测前缀 + 未测尾部**。`context_size.measured` 记下 provider 报回来的真实用量与当时的消息条数（`src/agent/contextSize/contextSizeOps.ts:45`，transient 不落盘），`get(start,end)` 对已测前缀用真值、对尾部用估算（`src/agent/contextSize/contextSizeService.ts:46`）。
- 缓存前缀**没有**编译器层的保护，只有 provider 层的断点：Anthropic 适配器把 `cache_control` 打在 system 上、最后一个 tool 上、以及最后一条消息的最后一个 block 上（`src/kosong/provider/bases/anthropic/anthropic.ts:852,975,877,299`）。也就是说断点跟着尾巴走，前缀是否稳定完全取决于上面那些插件有没有动它。

**对 sage 的判断**
- `抄`：**投影是纯函数 + 修复必留痕**。这就是备忘要的 View seam 语义：视图交出来的地方检查一次配对；kimi 更进一步 —— 不是断言崩掉，而是修好并把「修了什么」计数上报。sage 的清单天然能容纳这类 reason。
- `改造后抄`：**多投影变体应对 provider 拒绝**。413 降媒体、格式不合抹媒体，都是「同一份 Log 换个编译参数再来一次」的现成先例，sage 可以把它表达成一次预算更紧的重编译，而不是再写两个特判函数。
- `抄`：**已测前缀 + 估算尾部** 的记账法。备忘的成本不等式需要真数，估算器撑不住；这是拿到真数的最小结构。
- `改造后抄`：**`canSplitAfter`**。它就是备忘「块只能整进整出」的判据，而且已经把「未闭合的 tool 交换」这个最容易翻车的边界写成了一个谓词。sage 应该把它下沉成块边界的定义，而不是压缩策略的私有函数。
- `抄`：**user 提问逐字保留 + 摘要自带「这是笔记不是证据」前言**。前者正好是 pinned claim 的语义，后者几乎零成本地压住「摘要被当既成事实」的幻觉。
- `不抄`：**阈值触发压缩**。备忘指名的病灶：这一刀划不划算和什么时候挨到阈值无关。sage 的阶段边界必须由不等式批准。

---

## 3. 工具暴露

**机制**
- 默认全量给。开了 `select_tools` 渐进披露之后：可延迟的工具（MCP 工具，或声明 `disclosure: 'deferred'` 的）**只报名字**，以 `<tools_added>/<tools_removed>` 的**增量公告**进入历史（`src/agent/toolSelect/toolSelectService.ts:144`、`dynamicTools.ts:114`）；模型用 `select_tools` 按精确名字取全量 schema，schema 作为一条带 `tools` 字段的 system 消息**追加**进历史（`toolSelectService.ts:112`），之后 `shapeTools` 才把它列进 provider 的 tools（`toolSelectService.ts:86`）。
- **没有第二份账本**：「已加载哪些工具」就是历史本身，靠 `foldAnnouncedToolNames` / `collectLoadedDynamicToolNames` 折出来（`dynamicTools.ts:84,68`），所以撤销、压缩、恢复都自愈；文件头把这条设计意图写死了（`dynamicTools.ts:1-28`）。
- 工具结果截断在**工具自己那一层**，每个工具自带上限并告诉模型怎么拿更多：grep 分页给 `offset=`（`src/os/backends/node-local/tools/grep.ts:353`）、ripgrep 输出 10MB 封顶（`runRg.ts:21`）、glob 提示「换更精确的 pattern」。
- bash 是**留桩**而不是占位符：前台输出一旦被截断就把全量落到任务文件，并在结果尾部附上 `task_id / output_path / output_size_bytes / next_step: 用 Read 翻这个文件`（`src/os/backends/node-local/tools/bash.ts:425`）。
- `toolDedupe` 在执行前否决同一 step 内的重复调用，塞一条合成结果（`src/agent/toolDedupe/toolDedupeService.ts:136`）；跨 step 的连续重复则升级为三级 system-reminder，第 12 次直接强停（`toolDedupeService.ts:56,276`）。

**对 sage 的判断**
- `抄`：**工具目录的「一行档」已经有人跑通了**。名字增量公告 = 备忘里那句「一百行的工具目录降成一行，模型仍然知道有什么可用」；`select_tools` = expand。sage 把它表达成 claim 的 `tiers.line / tiers.full`，语义一模一样，还多了预算裁决。
- `抄`：**bash 的留桩格式**。备忘专门区分了「占位符 = 你可以再调一次」和「留桩 = 原文还在，按编号取」，kimi 已经实现了后者；`output_path + next_step` 就是现成的留桩渲染模板。
- `改造后抄`：**账本即历史**。「不要第二份真相」的立场和 sage 一致，但 kimi 为此把工具 schema 永久写进了 Log，且加载一个工具就改了 provider 的 `tools` 数组 —— 缓存断点当场作废。sage 该保留立场、换载体：账本从 Log 折出，但**呈现**留在视图里，工具目录的变更只在阶段边界生效。
- `不抄`：**重复调用的三级催促与第 12 次强停**。备忘「明确不做」里写着：这份设计只管这一轮发什么，不管要不要再来一轮。真要防打转，先看 §4 的确定性淘汰。

---

## 4. 子 agent 与并行

**机制**
- 子 agent 是**另一个 Agent scope**，有自己的 `wire.jsonl`、自己的 profile 与模型绑定（`src/session/subagent/tools/agent.ts:1-24`）。父给它的只有一句 prompt；`runAgentTurn` 是一个纯函数，借目标 scope 的几个服务跑一轮，然后按 profile 的 summary 策略从它自己的上下文里蒸一段摘要回来（`src/session/subagent/runAgentTurn.ts:1-18`）。父看到的只有这段摘要 + usage。
- 并行是 swarm：`AgentRunBatchLauncher` 做爆发-限流的启动坡道与 provider 限流恢复（`src/session/swarm/agentRunBatch.ts:1-10`），AgentSwarm 调用被强制独占一个 batch（`src/agent/swarm/swarmService.ts:1-21`）。
- 读/写分离不在 agent 这一层，而在**工具声明的资源访问**上：每次执行声明 `ToolAccesses`（read/write/readwrite/search + 是否递归），宿主调度器据此让互不冲突的调用并发（`src/tool/toolContract.ts:9,133-180`）。
- swarm 模式进/出用 Op 记录；退出时那条提醒的删除是 `ContextModel` 上注册的**跨模型 reducer**，重放时自动重现（`contextOps.ts:111`、`swarmService.ts:1-21`）。

**对 sage 的判断**
- `不抄`：**子 agent 从零起步 + 只回来一段摘要**。备忘要的是「换个问题、换个预算，在同一份 Log 上再编译一次」；kimi 这条路的代价是父的检索结果、已读文件、已定决策全部要靠 prompt 重说一遍，而子 agent 的发现只以摘要回流，块编号在两边指不到同一份原文。
- `抄`：**`ToolAccesses` 资源声明驱动并发**。备忘提到「七个第一方工具上已经声明好的危险度元数据」，kimi 把它做成了结构化的资源集合 + 冲突谓词，正好能给 sage 的后台只读编译（审查 / eval / 跨模型复看）当准入判据。
- `改造后抄`：**跨模型 reducer**。用来让「某个域的 Op 顺带改另一个域的状态」不必再写一条记录。sage 若把清单、块状态分成不同 Model，这是避免记录膨胀的现成招法 —— 但要守住备忘那条线：内核不得因此认识块的语义。

---

## 5. provider 层

**机制**
- 类型面是**自己的**，不是任何 SDK 的：`kosong/contract` 只有纯类型与纯函数，「no other domain, no I/O, no SDKs」（`src/kosong/contract/message.ts:1-11`），而且这条纯度由 CI 脚本强制（见 §8）。provider 适配器负责编解码到各家线上格式。
- 分层：contract(L0) ← protocol(L1) ← provider/model(L2) ← catalog(L3)，`model` 可以 import `provider`，反过来不行。适配器 base 不许 import 注册表，注册只在 `*.contrib.ts` 与目录 barrel 里发生（import 即注册）。
- 差异被归一化成显式契约：`FinishReason` 把各家停止原因映射成 7 个值并保留 `rawFinishReason` 逃生口（`packages/kosong/src/provider.ts:56-85`）；`ThinkingEffort` 是开放字符串 + 两个保留值；`TokenUsage` 统一成 `inputOther / inputCacheRead / inputCacheCreation / output`（`src/kosong/contract/usage.ts:12,17`），各家的缓存字段在适配器里抹平（`anthropic.ts:621,765`）。
- 模型装配集中在 `ModelCatalog`：一个地方解析 provider/model 配置、auth 闭包、endpoint 与 capability，「凡是带厂商形状的都走注册表，不许硬编码 switch」，并带一个按配置变更事件失效的缓存（`src/kosong/model/catalogService.ts:1-33`）。还有 `inspect`：同一条解析路径产出**带来源标注**（config / override / builtin / env / synthesized）的结果给 inspector 用。
- 流式与中止：streamed part 事件转发给调用方 `onPart`，abort 有统一的 `isAbortError / isUserCancellation / linkAbortSignal` 一套工具；失败分类成可重试/请求过大/图片格式等，驱动 §2 的投影重建（`src/agent/llmRequester/llmRequesterService.ts:1-28`）。

**对 sage 的判断**
- `抄`：**contract 层零 SDK 依赖，且由脚本强制**。备忘说阶段边界零代价换模型是 sage 的特色之一；这条只有在类型面属于自己时才成立，而「靠自觉」保不住，必须有闸门。
- `抄`：**usage 统一成四元组 + 保留 raw 逃生口**。成本不等式的两个倍率需要 `cacheRead / cacheCreation` 分得开，这是最小可用的字段面。
- `改造后抄`：**catalog 的 `inspect` 带 provenance**。「每个值为什么是这个值」和备忘「不允许有不知道为什么在那里的 token」是同一种洁癖，可以直接搬去做清单里 claim 的来源标注。
- `不抄`：16 个 provider 的适配广度本身。sage 第一天只需要两三个，但**契约形状**（FinishReason / ThinkingEffort / usage）要一次到位，否则以后每加一家就改一次内核。

---

## 6. 扩展机制

**机制**
- 内部扩展点是**有序 hook 槽**：`OrderedHookSlot.register(id, handler, {before|after})`，责任链式（handler 拿到 `next`，可以不调用 = 拦截），目标 id 不存在直接抛错（`src/hooks.ts:43,62`）。`before` 与 `after` 同时给也抛错。
- 服务本身是 DI + 三层作用域（App / Session / Agent，`src/_base/di/scope.ts:12`），`registerScopedService(scope, id, ctor, Eager|Delayed, domain)` 在 import 期注册。相当多的「插件」是**自布线**的：构造函数里把自己挂上 hook，别人不用 inject 它（`toolDedupe`、`toolSelect`、`swarm` 都是这样）。
- 往上下文里加东西走 `contextInjector`：插件注册一个 provider，返回一段文本或 content parts（`src/agent/contextInjector/contextInjector.ts:11-26`），服务在 `onWillBeginStep` 之后注入，并跟踪注入位置、在 wire 恢复后重新对齐（`contextInjectorService.ts:44,67`）。注入内容以 `origin: {kind:'injection'}` 的**真实消息**写进历史。
- 外部扩展：plugin（zip/GitHub 安装，带 manifest、skills、MCP server、命令）、skill（激活是一条 transient Op，`src/agent/skill/skillOps.ts`）、external hooks（把内部 hook 翻译成用户配置的本地命令，`src/agent/externalHooks/externalHooksService.ts:1-18`）。
- 注入进来的不可信文本一律转义（`escapeUntrustedText`，`src/agent/goal/injection/goalInjection.ts`）。

**对 sage 的判断**
- `改造后抄`：**顺序声明化 + 目标缺失即抛**。备忘把 harness-pi 的「顺序铁律只写在注释里」记成 G2；kimi 的答案是让顺序进代码并 fail-fast。sage 的主路径靠单一 view provider 消掉顺序问题，但工具执行、审批这些位置仍需要链，这时照这个形状做。
- `抄`：**自布线插件**。一个能力挂上去不需要装配器改四处，正好回应备忘吐槽的「加一个能力要改四处、装配函数 215 行」。
- `不抄`：**注入内容写进 Log**。这是 kimi 最贵的一处结构债：临时提醒一旦成为历史的一部分，撤销要按 origin 跳过它（`contextOps.ts` 的 `isRealUserPrompt`）、退出 swarm 要把它 pop 掉（跨模型 reducer）、渐进工具的协议消息要在 project 之前先 strip 掉 —— 一串特判全是这个决定的利息。sage 的 claim 天然长在视图里，不要走回去。
- `改造后抄`：**不可信文本转义**。零成本，第一天就该有。

---

## 7. 可观测

**机制**
- **每次 LLM 调用都落一条清单**：`llm.request` Op 记 provider / model / alias / thinking / temperature / topP / maxTokens / 是否开渐进工具 / `systemPromptHash` / `toolsHash` / `messageCount` / turnStep / attempt / 用了哪个投影变体 / 丢了几条（`src/agent/llmRequester/llmRequestOps.ts:53`）。它是**内容寻址**的：system prompt 只有和 profile 默认值不同才存全文，工具全量 schema 按 hash 去重、一份只存一次（`llmRequestOps.ts:42`，写入点 `llmRequesterService.ts:613`）。
- **同一份 Log 编译出第二份视图**：`reduceContextTranscript`（`src/agent/contextMemory/contextTranscript.ts:75`）镜像活路径的 reducer，**唯独 `context.apply_compaction` 保留全量历史、只追加一个摘要标记**（`packages/kap-server/src/services/snapshot/snapshotReader.ts:10`）——所以被压缩掉的回复在 UI 上仍看得见。给人的视图和给模型的视图规则不同、来源同一。冷会话则用 `groupTurns` + `foldFacts` 两级折叠从 `wire.jsonl` 重建整棵 turn 树（`packages/transcript/src/history/groupTurns.ts:1-28`）。
- 视图**档位**在订阅侧也存在：`off / turn / block / delta`（`packages/transcript/src/granularity/grade.ts:19`），降档直接丢在途内容、靠下一次整状态 upsert 收敛，升档由服务端从 L1 重发快照 —— 文件头写明「filter 本身绝不发明第二条投影路径」。
- 有独立的 inspector app（`apps/kimi-inspect`）：按 scope 列出每个服务的数据与触发按钮，transcript 面板能回放「当前这份 UI 状态是怎么一步步拼出来的」，带结构化 diff。
- 定位用 `LLMRequestTrace.traceId`（provider 的 `x-trace-id`）把一次未完成的请求和它的结果串起来（`src/kosong/contract/requestTrace.ts`）。
- 缺口：**没有回放控制器**。Log 能重建「当时的历史」和「那次请求的配置指纹」，但换一套策略把历史重编译一遍、比对模型下一步动作，这件事仓库里没有。

**对 sage 的判断**
- `抄`：**清单内容寻址**。sage 的 manifest 每轮都写，若存全文会把 Log 撑爆；`systemPromptHash` + 只在偏离默认时存全文、`toolsHash` + 首见才存 schema，是把「每次调用留一份清单」变得可负担的关键手法。
- `抄`：**为人的视图和为模型的视图用同一份 Log、不同 reducer**。备忘的「同一份 Log 上再编译」在这里已经有产品级先例，而且恰好证明了「压缩是视图的事、不是历史的事」。
- `改造后抄`：**订阅档位 + 降档靠下一次整状态收敛**。sage 的四档是给模型的，这套是给订阅者的，但「升档必须重发完整快照、绝不发明第二条投影路径」这条纪律两边都该守。
- `抄`：**traceId 贯穿未完成请求**。清单要能和 provider 侧日志对上，否则「它当时看到了什么」只答得出一半。

---

## 8. 工程手艺

**机制**
- **架构约束是可执行的**：`scripts/check-domain-layers.mjs` 检查三条规则 —— v2 不许 import v1、域只能 import 同层或更低层、kosong 子树的更严规则（contract 不许 import 任何外部包、provider base 不许 import 注册表）；层表就写在脚本里的 `DOMAIN_LAYER` map（`packages/agent-core-v2/scripts/check-domain-layers.mjs:53`），并且脚本本身被单测覆盖（`test/lint/domain-layers.test.ts`）。同目录还有 `op-uniqueness.test.ts`、`vendor-name-gates.test.ts`。
- **wire 协议有生成的清单**：`scripts/gen-wire-manifest.mts` 静态扫描 + 运行时排空 `OP_REGISTRY`，产出 `docs/wire-manifest.d.ts`（44 个 record 类型，每条标注属主 model、persisted/transient、是否发事件、是否有 blob 卸载、payload 形状），带 `--check` 新鲜度模式，并由 `test/wire/wireManifest.test.ts` 守着。
- **测试的默认断言就是「模型看到了什么」**：`ctx.lastLlmInput()` 把一次调用渲染成 `system: / tools: / messages:` 的文本快照；当这次的历史是上次的**前缀**时，只打印 `<last>` 加增量（`test/harness/snapshots.ts:154-174,194`）。system prompt 或 tools 没变就整段不打印。也就是说，**任何破坏前缀稳定性的改动都会让 inline snapshot 从「<last> + 两行」炸成整段历史**。
- 故障可点名注入：`faultInjection` 用实验 flag 守着，武装一次性故障让下一次请求在**接触 provider 之前**抛出指定错误，于是 413 / 图片格式两条恢复路径能端到端跑通（`src/agent/faultInjection/faultInjection.ts:1-17`）。
- 文件头即契约：几乎每个源文件开头都有一段结构化注释 —— 这个域是第几层、这个服务拥有什么、绑在哪个 scope、哪些东西**故意**不放进 Model、哪些注释是已知限制。`AGENTS.md` 里写着「以代码而非文档为真相源，除非用户明说，不要为了理解实现去读普通 Markdown」。
- CI：build + smoke + 5 路分片测试；PR 要求 conventional commit 标题、填模板、跑 `gen-changesets` skill 生成 changeset，且 **`major` 必须先问人**；禁止提交 agent 的交接/总结文档与临时原型文件。

**对 sage 的判断**
- `抄`：**前缀感知的 LLM 输入快照**。这是整个仓里对 sage 最直接有用的一件：备忘第③期要验「阶段冻结有没有真的成立」，这套快照让它变成默认断言，第一天就能用，不用等指标接进来。
- `抄`：**分层/纯度用脚本 + 单测守**。备忘的红线是「内核只拥有 Log 与 View seam，块、档位、预算、申领单全活在插件里」，这条线一旦只靠注释，第二个插件就会破。把「内核不许 import 编译器相关模块」写成 check 脚本。
- `抄`：**生成的 wire 清单 + 新鲜度检查**。Log 是 sage 的地基，「这个字段什么时候进来的」必须能一眼看完，而且不能靠人手维护。
- `改造后抄`：**文件头即契约**，特别是「哪些东西**故意**不进 Model」这类反向说明 —— 对一个「凡进视图必有 reason」的系统，这种负向记载同样重要。
- `不抄`：v1/v2 双引擎长期并存（`agent-core` 163k 行 + `agent-core-v2` 196k 行 + `migration-legacy`，加上 wire 1.0→1.5 五道迁移与一堆 legacy 形状兼容分支，如 `contextOps.ts:154-200` 的三态压缩 payload 联合）。sage 是从零重写，第一天就该只留一种形状；兼容负担是 kimi 的历史，不是 sage 的起点。

---

## 最值得搬的三件

1. **Op + 纯 reducer 的 Log**（`src/wire/op.ts:62,104`、`src/wire/wireService.ts:245`）：撤销与压缩都是追加，状态全由重放算出，内核照样不认识任何 Op 的语义。
2. **前缀感知的 LLM 输入快照**（`test/harness/snapshots.ts:154-174`）：`<last>` + 增量，让「阶段冻结成立了吗」从指标降级成默认单测断言。
3. **内容寻址的调用清单 + 第二份 reducer**（`src/agent/llmRequester/llmRequestOps.ts:42,53`；`src/agent/contextMemory/contextTranscript.ts:75`）：每次调用留痕但不撑爆日志，同一份 Log 编译出「给人的」和「给模型的」两份视图。

## 最该避开的三件

1. **把临时注入写进 Log**（`src/agent/contextInjector/contextInjectorService.ts:44`）：撤销、swarm 退出、渐进工具协议三处特判全是这一个决定的利息；claim 就是为了不走这条路。
2. **fork = 复制 wire.jsonl**（`src/app/sessionLifecycle/sessionLifecycleService.ts:363,502`）：备忘裁决①算过的那笔重复放大；子 agent 该是再编译一次，不是再存一份。
3. **阈值触发的有损压缩 + 催促/强停驱动**（`src/agent/fullCompaction/strategy.ts:18,110`；`src/agent/toolDedupe/toolDedupeService.ts:56,276`）：什么时候付缓存这笔钱要算不要撞阈值；要不要再来一轮不归这份设计管。

## 未核实

- v1（`packages/agent-core`，163k 行）没有读，本报告所有结论只覆盖 v2。
- 没有跑过这个仓的任何命令（只读侦察），CI 通过率、`gen-wire-manifest --check` 实际行为均未实测。
- `kap-server` 只读了 `snapshotReader.ts` 的文件头与 `reduceContextTranscript` 的调用点；REST/WS 表面未核实。
- 「缓存断点跟着尾巴走，前缀稳定性无人保护」是读 `anthropic.ts:852,975,877` 三处注入点 + agent-core-v2 全仓 grep 无 `cache_control` 得出的判断，未做运行时验证。
