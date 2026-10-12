# 参考仓侦察：deepseek-harness

- **是什么**：DeepSeek AI 开源的编码 agent 运行时 `dsh`，"everything-is-a-plugin"，跑在 Cordis 插件内核上；247 个包的两级 monorepo（`packages/<组>/<包>/`），另有 CLI 与 Web 两个 app。
- **规模**（实测）：`packages/*/*/src` + `apps` 的 .ts/.tsx 共 **298,468 行**；含测试的全部源码 763,201 行；1010 个测试文件；14,226 次提交。
- **主语言**：TypeScript（ESM、`strict`），少量 Python（SDK）。
- **最后活跃**：2026-08-28（`cd5ef81`，master，release/dsh-0.1.2-alpha.1 合并）。
- **与备忘观点的关系**：**它已经把"Log 是真相、发给模型的内容是投影"这条做成了内核**——`Session` 是只追加事件日志，`deriveMessages()` 每次从 surface 投影出消息，没有第二份 `_messages[]`。它缺的正是备忘的后半段：没有预算 claim、没有打分器、没有编译清单，压缩仍然是"到阈值就把前段总结掉"。所以它对 sage 的价值是：**原语这一层可以直接对照抄，策略这一层是留给 sage 的空地**。

---

## 镜头 1 · 状态与历史

**机制**

- 真相是只追加的事件日志。`Session` 持有 `log: SessionEvent[]`，`append()` 逐层校验并冻结 payload（拒绝 BigInt / 循环 / 稀疏数组），事件类型表见 `packages/core/session/src/types.ts:228-324`（`turn/start`、`step/start`、`user/message`、`assistant/chunk`、`assistant/message`、`tool/call`、`tool/result`、`request/header`、`request/context`…）。
- **消息历史是导出物**：`deriveMessages()` 在 `packages/core/session/src/index.ts:724` 沿 surface 节点折叠 `deriveEventMessage`（`packages/core/session/src/surface.ts:83`），每个节点只投影一次，返回每次新建的数组但共享深冻结的 `Message`。
- **"改写"是遮蔽，不是删除**：surface 事件带 `surfaceOp`，取值 `'append'` 或 `{op:'replace', start, end}`（`surface.ts:185-205`）。replace 把被遮蔽的节点从**投影**里摘掉，原始事件仍在 log 里；`replaceGeneration` 单调自增（`surface.ts:370`），投影缓存据此整体重建（`index.ts:728-732`）。
- **resume**：持久化后端订阅 `session/event`、在 `session/flush` 落盘（`packages/session/session-persistence/README.md`）；`session-checkpoint-policy` 在"模型请求发出前 / 顶层工具产生副作用前 / 每个 step 边界"做 fail-closed 检查点。崩溃日志由 `packages/core/session/src/repair.ts:27 interruptedTurnClosers()` 冷修复：给悬空 tool call 补合成结果，且**按风险分两种语义**——`TOOL_NOT_STARTED`（未记录启动，可重试）与 `TOOL_OUTCOME_UNKNOWN`（已启动但结果未落盘，要先验外部状态，"Do not retry blindly"）（`repair.ts:13,16`）。
- **分支**：只有 `ctx.sessions.fork(source, boundary?, childId?)`（`index.ts:1079`），按 seq 切一个前缀、要求切点不在 open turn 内（`index.ts:1130`），把事件**复制**进子 session 并带 lineage。README 明说"No session tree beyond fork"，没有回退/撤销。
- **fail-closed 词表**：`KNOWN_SESSION_EVENT_TYPES`（`known-event-types.ts:18`，由脚本生成）——读到本 build 不认识的事件类型直接拒绝重建，而不是跳过。理由见 `.agents/notes/implemented/simplification/2026-08-25-fail-closed-session-event-vocabulary.md`。

**对 sage 的判断**

- `append-only log + 投影出消息 + replace 只遮蔽不删` → **抄**。这正是 sage 的 Log 原语，且已被 14k 次提交验证过；`replaceGeneration` 给 View 缓存提供了唯一正确的失效信号。
- `repair.ts` 的**按风险分级的悬空 toolCall 合成结果** → **抄**。备忘里"R1 悬空 toolCall 由 seam 上一条断言堵死"只解决了不崩；这里还解决了"模型该不该重试"，而且答案写在模型看得见的文本里。
- fail-closed 事件词表 → **抄**。Log 是唯一真相的代价就是读错一个事件等于读错整个会话，宁可拒绝。
- `fork()` **复制事件** → **不抄**。sage 的第三样能力是"子任务不复制历史"，一旦 fork 复制 log 就等于自己放弃了这条；sage 的子 agent 应该是"同一份 Log + 另一份 claim 预算 + 另一次编译"，fork 只在真要独立演化时才复制。

---

## 镜头 2 · 发给模型的内容怎么构造

**机制**

- **system prompt = 有序 section 拼接**。`PromptSection` 带数值 `order`，同 order 按名字的 code-unit 序稳定排（`packages/core/system-prompt/src/index.ts:61,227`）；一方插件的位次集中写在 `FIRST_PARTY_SECTION_ORDER`（`index.ts:130`，`HARNESS_IDENTITY:-1000` … `TOOL_*:1000+`）。工具顺序另有 `orderTools()`（`index.ts:205`），配置里用 `TOOL_ORDER_REST` 占位。
- **动态上下文不进 system prompt，进历史，而且只在变了才写一条**。`renderContextSections()` 产出命名片段（`index.ts:302`），`RuntimeContextProjection.project()` 只在渲染结果**与上次保留的快照不同**时才返回一条新的 user 消息（`packages/core/agent-loop/src/runtime-context.ts:64-75`），旧快照被 replace 遮蔽后状态置空（同文件 :50-54）。时间、tmux 位置这类易变事实因此不会每轮改写前缀。
- **每次请求的信封被完整落盘**。`buildRequest()` 把 `{config, adapterDefaults, system, tools}` 规范化成 `EpochHeader`，首次/resume 写 `reason:'initial'|'resume'`，变了写 `'change'`，没变但开启新消息序列写 `'series'`（`packages/core/agent-loop/src/agent.ts:496-516`）。**surface 发生过 replace 就自动 `startsSeries`**（`agent.ts:503-504`）——缓存前缀失效这件事在 log 里有名字。`foldRequestHeader()` 取最新快照即可重建任何一次请求。
- **压缩**：`compaction-basic` 挂在 `agent/pre-step` 上按 `thresholdRatio × contextWindow` 判压力（`packages/compaction/compaction-basic/src/index.ts:147`），另有一条 `agent/request-error` 上的 `CONTEXT_WINDOW_EXCEEDED` 兜底重试路径（`index.ts:179-223`）。选区 `selectCompactableRange()` 从尾部倒着累加 token 直到满足 `retainTokens`，然后**往前退到一个 tool-call/result 成对平衡的边界**才下刀（`region.ts:100-136`，`region.ts:330,334` 是手动 `compactRegion` 的同名断言）。
- **缓存前缀是被有意保护的，而且有一条专门的裁决**。`.agents/notes/implemented/bug-fix/2026-07-21-compaction-summary-prefix-cache-reuse.md`：总结这次辅助调用原先用自己的 system prompt，第一个 token 就和热请求分叉，等于把最大的那次历史全价重算两遍。做法改成——**把总结指令从请求头挪到对话末尾的一条 user 消息**，system / tools / 历史逐字复刻热请求，于是辅助调用是热请求的真前缀延长。裁决里还明写"tools 也要带上，哪怕总结器一个都不调，去掉会缩短 token 序列、破坏对齐"。
- 没有 `cache_control` 之类显式标记（DeepSeek 侧是自动前缀缓存）；`usage` 里拿 `cacheReadTokens` / `cacheWriteTokens`（`packages/llm/llm/src/types.ts:146`）。

**对 sage 的判断**

- **动态上下文"只在变了才追加一条"** → **抄**。这是不引入编译器也能拿到的缓存红利，sage 的 `pinned` 进度账本正该用同一条规则：账本内容没变就不写新条目。
- **`request/header` 作为可重建信封 + surface 变更自动开新序列** → **改造后抄**。sage 的"编译清单"比它信息更多（claims / 淘汰 / 档位），但"信封落进 Log 且能 fold 出最新一份"这个形状可以直接搬；`startsSeries` 就是 sage 的 `epoch` 递增，改造点是把触发条件从"surface 变了"换成"成本不等式判定要开新阶段"。
- **辅助调用必须是主请求的前缀延长** → **抄**。sage 打算把打分器换成便宜模型，同一个陷阱等着：打分调用如果自带 system prompt，就会在最该省钱的时刻全价重算。规则可以更强：**任何辅助调用只许在主视图末尾追加，不许改头**。
- **压缩前先退到 tool 配对平衡边界** → **抄**。sage 的内核只保证"视图里 toolCall 成对"，那么淘汰器必须自己知道什么是可切点，否则断言会在编译器里炸而不是在策略里炸。
- `thresholdRatio` 单阈值触发 + 一次性总结前段 → **不抄**。这正是备忘要否掉的那条路：有损、一次性全前缀失效、总结完就问不出"刚才那文件第 80 行"。sage 的档位 + 留桩 + expand 是它的替代品。

---

## 镜头 3 · 工具暴露

**机制**

- 默认全量：插件 `ctx.tools.register()`，schema 自动流进 prompt 组装（`packages/core/tools/README.md`）。没有延迟加载、没有工具搜索；唯一的减法是按 scope 隐藏和配置 `toolFilter`。（`llm-pi-ai/src/catalog.ts:240` 的 `deferredToolsMode:'withhold'` 是 pi-ai 库自己的字段，不是 dsh 的延迟工具机制——**未核实**其实际效果。）
- **PTC 模式是它对"工具目录太占位"的回答**：`mode: 'ptc'` 时模型只看到一个 `run_code` 工具加一段生成的 SDK 类型声明，所有工具变成程序里的 `await tools.name(args)`；每次绑定调用仍然走完整的工具管线并带相关性日志。关键句（`packages/core/tools/README.md` 的 PTC 节）："只有你打印或返回的才是程序输出……**其他中间结果一律不进对话**"。`both` 模式两套并存；`ptc` 独占时直呼其他工具返回 `UNKNOWN_TOOL`，"宣告的表面和可调用的表面保持一致"。
- **结果截断是一个共享库，且如实报账**：`packages/util/output-retention` 的 `ItemRetainer`（按条数留头）/ `TextRetainer`（按字节留头、留尾或头尾，切口保证 UTF-8 合法），`finish()` 返回保留内容 + 精确的 `omitted` 计数；库只回答"留了什么、丢了什么"，恢复指引归工具自己。
- **超大结果落盘 + 留位**：`spill-policy` 在最终结果超过 `maxInlineBytes` 时把全文存成 session 私有工件，模型侧换成头尾预览加一行 `(Omitted N bytes. Full formatted result stored at: <locator>. <retrievalHint>)`（`packages/spill/spill-policy/src/index.ts:104-176`）。实现细节值得记：**通知那行的字节成本是在 cap 内部预留的**（`src/index.ts:163-175`），所以替换体永远不超过声明的上限；存储失败则原样保留，绝不因为落盘失败而丢内容。
- **重取**：`tool-session-query` 给模型五个只读工具 `session_search` / `session_event_search` / `session_trace` / `session_event_trace` / `session_event_read`（`packages/session-query/tool-session-query/src/index.ts:67-110`），按 workspace 授权（只能读 cwd 完全相同的会话），结果是无游标纯文本。它是 opt-in，shipped 组合里没挂。

**对 sage 的判断**

- **spill 的"预览 + locator + retrievalHint"，通知成本预留在 cap 内** → **抄**。这就是 sage 的留桩，且这条工程细节（不预留就会超标）是白捡的。
- **`session_event_read` 这类按 seq 回读原始事件的工具** → **抄**。sage 的 `expand` 需要一个落点，按 Log 的 seq 直接回读是最省事的那个；顺便 expand 率就有了分母。
- **PTC 模式"中间结果不进对话"** → **改造后抄**。备忘里"带 100 个工具的目录只占 100 行预算"是用 claim 解决的；PTC 解决的是另一半——**工具结果的量**。sage 可以把它做成一个 claim 档位之外的正交能力：编排型任务走 run_code，只有返回值进 Log。改造点是 PTC 把可调用面也换掉了，sage 不必这么激进。
- 全量工具目录 + 无延迟/无搜索 → **不抄**（但理由要说准）：dsh 是单产品自带电池，工具数可控；sage 的 claim 模型本来就允许工具目录降档到"一行"，比延迟加载更可控——延迟加载会让工具目录这段前缀在中途变化，反而砸缓存。

---

## 镜头 4 · 子 agent 与并行

**机制**

- 一个 `ctx.subagents` 契约 + 一堆命名 provider：`spawn`（进程内、空历史）、`fork`（进程内、种入父会话已完成的轮次）、`acp` / `codex` / `claude-code` / `dsh-sdk`（进程外，分别走 ACP、Codex app-server、Claude Agent SDK、自家 TS SDK）。
- **fork 的上下文 = 父会话"已完成轮次"前缀的一次性快照**：`completedTurnPrefix(request.parent)` 取到最后一个 `turn/end`，当前在飞的这一轮不给（`packages/subagent/subagent-fork-in-process/src/index.ts:76`）；之后父会话再长也不会同步给孩子。
- **结果回传**：一次性子 agent 直接把结果返回给发起它的那一轮；可续接的后台子 agent 通过 `tool-subagent-report` 拿到一个 child scope 专属的 `report` 工具，报告以 `Background subagent <child-id> reported:` 的形式变成父会话的普通消息。
- **这里有一条对 sage 极其重要的裁决**：`.agents/notes/implemented/architecture/2026-08-10-fork-children-stay-one-shot.md`。`report` 工具 + 它的 prompt 片段是**加在请求头**的（system 块和 tools 块在所有 provider 的线格式里都排在消息之前），所以"可续接的 fork 子 agent"会在第一条继承来的历史**之前**就把前缀改掉——它复制了历史却拿不到复用，"付了 fork 的成本、收不到 fork 的收益"。结论是所有出厂组合把 fork 绑成 one-shot，代码路径保留，条件写成 `TODO(fork-continuable-prefix-reuse)`（`src/index.ts:84-89`）：等子会话的 system prompt 与 tool schema 能和父会话逐字节一致时再开。裁决里还否掉了"把 child 的增量排到历史之后"——**不可表达**。
- 并行：后台工作走 `jobs` 家族（job 归属起它的 session，完成后在会话内投递而不是轮询）；`snapshots/session/subagent-parallel/` 有并行子 agent 的录制用例。没有看到"只读子 agent"这一类专门的权限分离；隔离靠 Cordis scope（`packages/core/scope`）与每个 delegation tool 的 `toolFilter` / `persona`。

**对 sage 的判断**

- **"子 agent 只要在请求头加任何东西，前缀复用就归零"** → **抄这条结论**。sage 的"零拷贝扇出"如果给子视图换 persona、换工具集、加一段子任务说明，收益就没了。正确形态是：**子视图与主视图共用同一个头（system + tools 逐字节相同），差异全部落在消息尾部的 claim 组装里**。这条要写进 sage 的 view seam 契约，而不是留给插件自觉。
- **`report` 作为 child scope 专属工具、结果变成父会话的普通消息** → **抄**。回传不需要新通道，就是往父 Log 追加一条事件；同时"只有 child scope 看得见 report"避免了根 agent 广告一个没有收件人的工具。
- fork = 复制已完成轮次前缀 → **不抄**（同镜头 1）。sage 的等价物是"同一份 Log、换一份预算和问题重编译一次"。
- 缺"只读 / 可写"分离 → **不抄它的缺失**：sage 的后台审查、eval、跨模型 review 都是只读消费者，这个维度得自己补。

---

## 镜头 5 · provider 层

**机制**

- `ctx.llm` 是唯一的 provider-neutral 服务；类型面属于 harness 自己（`packages/llm/llm/src/types.ts`、`message.ts`、`content.ts`）：`Message` / content block / `StreamChunk`（`block-start`、`text-delta`、`reasoning-delta`、`tool-call-delta`、`block-end`、`usage`、`finish`）。
- **适配器契约**：抽象类而不是 interface，`stream(options): AsyncIterable<StreamChunk>` 必须响应 `options.signal`（`packages/llm/llm/src/index.ts:274`）；`prepareCall(provider, model, signal)` 先解析模型再绑定一次 dispatch（`index.ts:262`），返回的 `PreparedAdapterCall` 带 `adapterDefaults`（哪些值是适配器补的、哪些是显式设的）和 `context.contextWindow`。
- **双实现验证**（`.agents/notes/implemented/architecture/2026-06-13-twin-llm-adapters.md`）：从第一天就跑两个结构完全不同的适配器打同一个契约——`llm-deepseek`（自己 fetch + SSE 翻译）和 `llm-pi-ai`（走 `@earendil-works/pi-ai` 库）。规则是"**StreamChunk 词表表达不了其中任何一个，就是核心词表的 bug**"。这一对钉死了几条约定：usage 必须在 finish 之前、finish 之后不许有东西、tool-call `arguments` 全程是原始 JSON 字符串、错误只有两条合法路径（`stream()` 抛，或以 `finish {kind:'error'|'aborted'}` 结束，`types.ts:120-124`）。
- 多 provider 靠配置而非代码：pi-ai 路由字典，pi-ai 没内置的 provider 直接声明出来，OpenAI 兼容网关是配置项；凭据/profile 每次请求解析，改用户设置文档即刻生效、无需重启。
- usage 字段：`inputTokens` / `outputTokens` / `totalTokens` / `cacheReadTokens` / `cacheWriteTokens` / `reasoningTokens`（`types.ts:131-146`），`token-meter` 从**持久日志**而不是内存状态度量压力。
- 失败重试 `llm-retry` 挂在 `agent/request-error` 上，只在**可持久化的 step 边界**重跑。

**对 sage 的判断**

- **双适配器当"契约验证孪生"** → **抄**。sage 的卖点之一是"阶段边界零代价换模型"，那么中立词表必须被第二个实现证伪过；只有一个适配器时，"中立"是自说自话。
- **两条且仅两条错误路径 + usage 必须早于 finish** → **抄**。这类约定写进类型注释并让两个实现同时遵守，比事后写 if 便宜。
- **`adapterDefaults` 标记"哪些值是适配器补的"** → **抄**。sage 的清单要回答"这次调用为什么长这样"，适配器偷偷补的默认值不标出来，清单就是假的。
- `prepareCall` 先解析再绑定 → **改造后抄**：sage 的阶段路由要在编译时就知道 `contextWindow`，这个两段式正好；改造点是让 `prepareCall` 的结果参与成本不等式，而不只是发请求。

---

## 镜头 6 · 扩展机制

**机制**

- 一切是 Cordis 插件。**没有"hook 系统"这个包**——裁决原话："native hook 不是一个包，它就是一个订阅了规范生命周期事件的普通插件；Claude Code / Codex 的 hook 桥只是把外部 shell 协议翻译到同一套 API"（`.agents/notes/implemented/feature/2026-06-30-interception-extension-points.md`）。桥能做的，普通插件都能做，而且更强（无序列化边界、拿得到完整 `ctx`、返回值有类型）。
- **三域边界规则**（`.agents/notes/implemented/architecture/2026-06-30-event-domain-semantics.md`）：`session/*` 是可重放的**事实**日志（纯 JSON，不放活对象）；`agent/*` 是**活的**运行时面（带 `Agent` 句柄，拦截用）；`tools/*` 是工具管线。一条判据："可重放的事实是 SessionEvent，活的拦截或瞬时信号是 Cordis 事件。" 据此他们**删掉了** `agent/turn-start|turn-end|step-start|step-end` 四个镜像事件——同一事实两个来源就是 bug。
- **顺序问题是用"权限形状"解掉的，不是用优先级数字**。工具管线固定为 `tools/pre-execute`（可 allow/deny/ask 的 waterfall）→ `ctx.tools.guard()`（同步、**只能 deny 或弃权，永远不能强制 allow**，所以监听顺序无法复活一个被禁止的操作）→ `tools/execute`（around 包装）→ dispatch → `tools/post-execute`（可替换内容或值、可 block、可附加上下文）→ `finalizeContent`（definition 自己的最后一道，只能改 content）→ `tools/result`（只读通知，监听器抛错被隔离，改不了结果）。
- waterfall 监听器**必须调 `next()`** 才算委派，直接返回就短路整条链（AGENTS.md 约定）。
- 契约与实现分包：**Service Definition / Service Provider / Consumer** 三角色（`.agents/notes/implemented/architecture/2026-06-13-capability-seams.md`）；换沙箱执行器不会动模型看见的 schema。明写"不要预先拆分——只有一个可能实现和一个消费者时就一个包"。
- 运行时自改：`extensions/` 让模型在跑的过程中定义 / 运行 / 卸载动态 Cordis 包（只在内存，重启即清）。

**对 sage 的判断**

- **"hook 不是一个包，是订阅规范事件的普通插件"** → **抄**。sage 只有 Log 和 View seam 两个原语，再长出第二套 hook 体系就是自废武功。
- **三域边界规则（可重放事实 vs 活的拦截）** → **抄**。这是 sage 最缺的那条判据：什么进 Log、什么只是运行时信号。没有它，Log 会被塞进活对象，回放立刻失效。
- **同一事实不许有两个来源（删镜像事件）** → **抄**。
- **用权限形状而不是优先级解决顺序**（guard 只能否决、通知层改不了结果） → **抄**。备忘吐槽"谁先谁后只写在注释里"，正解不是给插件排号，而是让每一段的**权限单调**，排序就不再是正确性问题。
- 数值 `order` 排 prompt section（镜头 2） → **改造后抄**：sage 的组装顺序应由 claim 的优先级与档位解出来，数值 order 只保留给"钉在最前的身份段"这种少数位次。
- 运行时动态定义插件 → **不抄**（至少第一天不）。它和"每次调用留一份可重放清单"直接冲突：回放时那个插件已经不存在了。

---

## 镜头 7 · 可观测

**机制**

- **"model-visible ⟺ logged"是写进 AGENTS.md 的硬规则**："任何进入模型请求的东西都必须能从 session log 重建；新增一个模型可见的输入就必须新增一个 session 事件。"
- 因此"模型当时看到了什么"是可计算的：`request/header` 存了 config + adapterDefaults + 渲染后的 system prompt + 组装好的 tool schema，`foldRequestHeader()` 取最新一份；消息侧把 `deriveEventMessage` 折过该前缀的 surface，得到与当时**逐字节相同**的 `Message`。压缩总结那次辅助调用就是靠这条重建前缀的。
- 每次调用还留了**逐 chunk**的 `assistant/chunk`（`turn`/`step`/`chunk`），加上 `assistant/message`（含 `usage`、`interrupted`）、`tool/call`、`tool/result`、`llm/retry`、`compaction/start|summary|prune|end`、`hook/invoked|result`。
- **回放是测试基础设施的默认形态**：`vitest.snapshot.config.ts:24-27` 注释说明——replay 是无 key 的默认档，从录制的模型响应启动真实子进程路径，**diff 组装出来的请求**、规范化后的协议/transcript 输出、以及持久化日志的期望输出；`record` 才打真 API，`refresh` 只重放已提交脚本并更新期望值。
- 每个场景一个目录，例如 `snapshots/session/compaction-recovery/`：`session.jsonl`（既是回放输入又是期望输出）、`snapshot.yml`、**`system-prompt.expected.md`**、**`tool-schemas.expected.json`**。也就是说"模型看到的 system prompt 和工具 schema"是**提交进仓库、代码评审时逐行看 diff** 的产物。`snapshots/AGENTS.md` 规定：易变身份替换成保留关系的 token，system prompt / tool schema 替换成 token 并由唯一一个 sidecar 持有可读全文；改动工作区的场景要提交完整结果目录当独立 oracle，"模型的散文和工具结果文本不能证明外部副作用"。
- 另有 `session-projection`（把 log 折成客户端要的当前值，带 `stateVersion` 供缓存失效）、`session-telemetry`（投影 + 脱敏 + 后端上报，并公开声明本部署的共享策略）、`session-log-deepseek`（把规范日志增量上传为官方 API 的 `dsh_session_log` 字段，用一个持久事件推导**acceptance watermark**）。

**对 sage 的判断**

- **"model-visible ⟺ logged"作为一条会被 review 执行的规则** → **抄**，而且第一天就写进 sage 的贡献约定。备忘的"可审计上下文"只有靠这条才成立。
- **把 system prompt 与 tool schema 作为提交进仓库的期望产物** → **抄**。sage 的编译清单更适合做这件事：清单本身就是快照。前缀变没变、谁多占了预算，在 PR diff 里直接看见。
- **keyless replay 作为默认测试档（录制一次，之后不花 key）** → **抄**。备忘里"上下文策略可离线回放"在这里已经是可运行的工程形态，不必从零设计：录制的会话 JSONL 同时当输入与 oracle。
- **acceptance watermark（增量上传只发未确认的后缀）** → **改造后抄**。sage 的 Log 不设水位（先落盘再进内存），这套只在清单流式外发、需要增量上传时才用得上。
- 逐 chunk 落盘 → **改造后抄**：调试期极有价值，但它让 Log 体积按 token 增长。sage 应把 chunk 归到"可裁剪的 trace 层"，与"编译输入层"分开，别让回放必须驮着它。

---

## 镜头 8 · 工程手艺

**机制**

- **Agent Note = ADR，且是硬要求**："非平凡改动必须在同一个 PR 里带一份 Agent Note"。仓库里 **814 份**（`.agents/notes/`，另有 zh 对照），按 `implemented / proposed / rejected / archived` × `architecture / feature / bug-fix / simplification / process / testing` 两维分目录，格式由 `verify-agent-note-format` / `verify-agent-note-classification` / `verify-archived-agent-notes` 三个门守；归档的笔记**冻结**，不许当现行权威引用。每份的结构固定：Problem / Decision / Alternatives considered / Consequences / Testing——上面引用的几条裁决之所以能直接用，就是因为"否掉了什么、为什么"都写了。
- **README 有强制章节契约**：242 个包 README 全部带 `## Model Experience`，其下必须回答 `#### What the model sees` / `#### Token effect` / `#### KV Cache effect`。由 `scripts/verify-package-readme-model-experience.ts:13-18` 当门，且**豁免要写理由并入库**（`NO_MODEL_EXPERIENCE_SECTION` / `SENTENCE_MODEL_EXPERIENCE` 两张带 reason 的表，`:31-60`），"缺席不能被误读成忘了写"。
- 测试分层：`test`（单测，CI 用 `test:coverage`，`packages/*/*/src` **逐文件 100%**）/ `test:e2e`（真 API，没 key 自跳）/ `test:expected`（进程级期望）/ `test:snapshot`（无 key 录制回放）/ `test:web` / `test:gui` / `duplication`（跨文件克隆检测）/ `hygiene`（knip + publint + workspace 约束）/ `doc-sync`（全部文档门）。179 个 `scripts/`，门由 `scripts/run-gates.ts` 聚合成命名档（`ci-primary` / `ci-static` / `ci-snapshot` / `ci-windows-*` …）。
- 其它写进 AGENTS.md 且会咬人的：注册即 effect（`ctx.effect()` / `ctx.on()`，`register()` 返回 disposer）；闭合 union 以 `assertNever` 结尾、可合并扩展的 union 走一个有文档的 default；不可变 id 用 `Branded<B>` 而不是裸 string；同进程有类型的边界**不要**加运行时校验（只在 parser/config、队列、模型与工具 JSON、持久化、worker、进程、线协议这些真边界校验）；空 `catch` 必须写清吞了什么；"测试描述行为不描述正确性，行为过时就连测试一起改，并在 PR 里说为什么"。
- 文档语气也有硬规矩："注释与文档陈述完整契约与上下文，不是推理过程；不用比喻；写 `contract` / `boundary` / `shape` 之前先问有没有更准确的词"。
- 预发布姿态写在最前面：**"foundation over blast radius"——首个 tag 之前不留兼容垫片，宁可改名重排并更新全部引用；后端直接拒绝旧磁盘格式。**

**对 sage 的判断**

- **Agent Note 四段式（含 Alternatives considered / Consequences）作为 PR 必带件** → **抄**，第一天就用。sage 的赌注全在几个非显然的设计选择上（前缀冻结、档位、claim），没有"否掉了什么"的记录，三个月后会被人当 bug 改回去。
- **README 的 `Model Experience / Token effect / KV Cache effect` 三问 + 带理由的豁免表 + CI 门** → **抄**，这是这次侦察里最值得直接搬的一件。sage 的每个插件都在花预算，这三问逼它把账写在自己门口；豁免要写理由这一条防止门被空转。
- **逐文件 100% 覆盖 + 分层命名的门（不是一坨 CI）** → **改造后抄**。sage 早期跑不起 100%，但"门按档命名、本地按改动面挑最小档"这个形状可以先有。
- **"预发布不留兼容层"** → **抄**。sage 要把 View seam 这类内核契约改到位，留垫片等于把备忘要删的管子换个名字留下。
- 179 个脚本 / 247 个包 / 两级 monorepo / 全仓双语文档 → **不抄**。这是一个有产品压力的大团队的均衡点；sage 的两个原语撑不起这个量级的分包，过早拆分会把"内核行数是减的"这句话变成反话。

---

## 最值得搬的三件

1. **README 的 `Model Experience`（What the model sees / Token effect / KV Cache effect）三问 + 带理由的豁免表 + CI 门**（`scripts/verify-package-readme-model-experience.ts:13-60`）——让每个插件在自己门口把预算账写清楚，正好是 claim 模型的文档面。
2. **keyless 回放做成默认测试档，且把 `system-prompt.expected.md` / `tool-schemas.expected.json` 提交进仓库**（`vitest.snapshot.config.ts:24-27`、`snapshots/session/*/`）——"模型当时看到了什么"变成 PR 里逐行可看的 diff，sage 的编译清单可以直接占这个位置。
3. **"辅助调用必须是主请求的前缀延长，连用不到的 tools 都要照带"**（`.agents/notes/implemented/bug-fix/2026-07-21-compaction-summary-prefix-cache-reuse.md`）——sage 的打分器、摘要器、子视图都要守这条，否则最省钱的那一刻全价重算。

## 最该避开的三件

1. **子视图在请求头上加任何东西**（`.agents/notes/.../2026-08-10-fork-children-stay-one-shot.md`）——system 与 tools 块在所有 provider 的线格式里都排在消息前，子 agent 加一个 `report` 工具就让继承来的历史全部重算，"付了复制的成本、收不到复用的收益"；而且"把增量排到历史之后"不可表达。sage 的零拷贝扇出必须把头钉死。
2. **fork 靠复制事件做分支**（`packages/core/session/src/index.ts:1079-1130`）——一旦子会话复制 Log，sage 的"子任务不复制历史"这条卖点就自己没了；子 agent 应是同一份 Log 上换预算重编译。
3. **单阈值触发 + 一次性总结前段的压缩**（`packages/compaction/compaction-basic/src/index.ts:147`、`region.ts:100`）——有损、前缀整段失效、事后问不出细节；这正是备忘要替换掉的那条路，别因为它"现成能跑"就先抄进来。
