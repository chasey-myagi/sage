# recon: pi-mono — 对 sage 的取材

- **是什么**：`earendil-works/pi` 的 monorepo（`pi.dev`），四个包：`tui`（终端 UI）/ `ai`（统一多 provider LLM 库，20+ provider）/ `agent`（agent 运行时 + 新的 `AgentHarness`）/ `coding-agent`（可自扩展的编码 agent CLI，即 `pi`）。
- **规模（wc -l 实测）**：`packages/*/src` 非生成 TS 79,377 行（另有 `models.generated.ts` 等 16,849 行）；`packages/*/test` 65,444 行（230 个 `*.test.ts`）；`packages/*/docs` 12,213 行 Markdown；599 个 `.ts` 文件，4,238 个 commit，起于 2025-08-09。
- **主语言**：TypeScript（纯 ESM，Node ≥ 22.19，biome + `tsgo --noEmit`）。
- **最后活跃**：本地 checkout 的 HEAD 停在 2026-05-21（`11c3da4` "fix(ai): set bedrock claude default max tokens"），工作树文件 mtime 2026-07-20。
- **与备忘的关系**：这就是备忘里 harness-pi 的**上一代形态**——「记录」和「发给模型的内容」在这里已经分家（`SessionStore` 是只追加的**树**，视图由 `buildSessionContext` + 一条 `transformContext` 每轮重算），但没有预算、没有清单、没有阶段冻结。备忘那句「最难的一步已经走完」，在这份快照里可以逐行核对。

> **版本差说明**：本 checkout 没有 `packages/core`，也搜不到备忘点名的 `microcompact` / `deferredTools` / `additionalContext` / `transformMessagesBeforeLlm` / 工具危险度元数据。备忘描述的是更晚的版本。下文只写这份快照里我真读过的东西；备忘里那些名字在这里对不上，属于**未核实**。

---

## 1. 状态与历史

**机制**

- 会话 = 一个 JSONL 文件，**每行一条 entry，靠 `id` / `parentId` 组成一棵树**，不是线性数组（`packages/coding-agent/docs/session-format.md:3`）。entry 类型有 `message` / `compaction` / `branch_summary` / `model_change` / `thinking_level_change` / `label` / `custom` / `leaf` 等（`packages/agent/src/harness/types.ts`）。
- `Session` 类只有 `append*` 方法，没有任何 update/delete：`appendMessage` / `appendCompaction` / `appendLabel` / `appendSessionName` 全部走同一个 `appendTypedEntry`（`packages/agent/src/harness/session/session.ts:123-230`）。
- **持久化先于内存**：`appendEntry()` 先 `fs.appendFile` 落盘，成功后才 `this.entries.push` + 更新索引（`packages/agent/src/harness/session/jsonl-storage.ts:250-258`）。所以不需要额外的水位——文件本身就是水位。
- **"当前在哪"也是一条 entry**：`setLeafId()` 追加一条 `leaf` entry 再改内存游标（`jsonl-storage.ts:226-243`）；文档把这条写成硬要求：「`setLeafId()` 不是内存游标，重开 storage 必须能从最后一条 leaf entry 重建位置」（`packages/agent/docs/agent-harness.md:82`）。
- **回退 / 分支**：`moveTo(entryId, summary?)` 把 leaf 挪到任意历史节点，可选地追加一条 `branch_summary` 记录"我从哪条分支走过来、那边干了什么"（`session.ts:232-251`）；读取路径是 `getPathToRoot(leafId)` 沿 `parentId` 上溯（`jsonl-storage.ts:275-289`）。分支**不复制任何 entry**。
- 唯一会复制的是 `SessionManager.forkFrom()`：把源文件的全部 entry 逐条写进新文件（`packages/coding-agent/src/core/session-manager.ts:1353-1394`），只在 header 里留一个 `parentSession` 指针。
- resume：`pi -c` / `pi -r` / `pi --session <id>` / `pi --fork <id>`（`packages/coding-agent/docs/sessions.md:9-15`），交互里是 `/resume` `/tree` `/fork` `/clone`。

**对 sage 的判断**

| 项 | 判断 | 理由 |
|---|---|---|
| 只追加 + entry 树（`parentId`）+ `leaf` 也是 entry | **抄** | 这正是 sage 的 Log 原语，而且比"数组 + 索引"更强：分支/回退不需要第二套机制，"当前编译入口"本身可回放。sage 的 Log 应该一上来就是树，别先做线性再补。 |
| 先落盘再进内存 | **抄** | 崩在半路时磁盘永远是内存的超集，不用 HWM 这个额外概念。sage 的 Log 只有 append，这条一行代码就能守住。 |
| `buildSessionContext` 把 compaction 当成"replay 时跳过一段"的指令（见镜头 2） | **改造后抄** | 方向对：压缩是 Log 里的一条记录，不是对 Log 的改写。但它写死在内核投影里；sage 该把它降级成编译器清单的一个特例（备忘：「压缩边界只是阶段清单的一个特例」）。 |
| `forkFrom` 整文件复制 | **不抄** | 裁决①点名过的放大：8 个分支 = 8 份全量重复。sage 的子 agent 是「在同一份 Log 上再编译」，永远不该有第二份 Log。 |

---

## 2. 发给模型的内容怎么构造

**机制**

- **只有一条接缝**：`agent-loop.ts` 在每次调 provider 之前调一次 `config.transformContext(messages, signal)`，拿回的数组再 `convertToLlm` 成 wire 消息，然后组 `{ systemPrompt, messages, tools }`（`packages/agent/src/agent-loop.ts:276-296`）。整个循环只有这一处能改"发出去的内容"。
- 接缝背后是**顺序可变的插件链**：`emitContext()` 先 `structuredClone(messages)`，再按**扩展加载顺序**逐个 handler 传递，谁返回 `{messages}` 谁就覆盖后续输入；handler 抛错被吞掉（只发一条 error 事件）并**带着上一版继续**（`packages/coding-agent/src/core/extensions/runner.ts:858-886`）。文档只说一句「Handlers run in extension load order」（`packages/coding-agent/docs/extensions.md:600-612`）。
- **system prompt 是每轮拼出来的字符串**：`buildSystemPrompt()` 依次拼 custom prompt / `appendSystemPrompt` / `<project_context>`（AGENTS.md 等）/ skills 目录 / **最后两行是 `Current date:` 和 `Current working directory:`**（`packages/coding-agent/src/core/system-prompt.ts:28-80`）。
- **压缩触发是纯阈值**：`shouldCompact = contextTokens > contextWindow - reserveTokens`，默认 `reserveTokens=16384`、`keepRecentTokens=20000`（`packages/coding-agent/src/core/compaction/compaction.ts:115-124, 219-222`）。
- **token 估算是"实测 + 估算"的混合**：取最后一条 assistant 消息的真实 `usage` 当基数，只对它之后的消息用 `chars/4` 估（`compaction.ts:186-213, 232`）。
- **切点规则是结构性的**：`findValidCutPoints` 允许在 user / assistant / bashExecution / custom / branch_summary 处切，**永不在 `toolResult` 处切**（`compaction.ts:299-336`）；`findCutPoint` 从尾部往回累加到 `keepRecentTokens`，落在 turn 中间就标 `isSplitTurn`，此时生成"历史摘要 + turn 前缀摘要"两份再合并（`compaction.ts:386-446`、`docs/compaction.md:81-107`）。
- **缓存前缀是 provider 层的事，不是 harness 的事**：`getCacheControl()` 按 model 元数据决定 ttl（`packages/ai/src/providers/anthropic.ts:54-67`），然后把 `cache_control` 打在三处——system prompt（`anthropic.ts:886-925`）、**最后一个 tool 定义**（`anthropic.ts:1185`）、**最后一条 user 消息的最后一个 block**（`anthropic.ts:1134-1155`）。model 上有 `cacheControl` 约定字段与 `supportsLongCacheRetention` 等能力位（`packages/ai/src/types.ts:394-432`）。
- **投影会自我修复**：每个 provider 都先过 `transformMessages()`——丢掉 `stopReason` 为 `error`/`aborted` 的 assistant 消息，给孤儿 toolCall 补一条 `"No result provided"` 的合成 toolResult（`packages/ai/src/providers/transform-messages.ts:155-218`）。

**对 sage 的判断**

| 项 | 判断 | 理由 |
|---|---|---|
| 循环里**只有一个**"改发出去的内容"的点 | **抄** | 这就是 View seam 的形状，而且已经被验证过够用：sage 把签名从 `(messages)=>messages` 换成 `view(log, turn)=>CompiledView` 即可，位置一模一样。 |
| 插件链按加载顺序串行改同一个数组 | **不抄** | 备忘的 G2：顺序只写在注释里、没人算总账、一个 handler 抛错整条链的语义就漂了。sage 用 claim + 编译器解一次分配，结构上没有"顺序"这个自由度。 |
| `structuredClone` 后再交给插件 | **抄** | 一行代码换来"插件永远改不到 Log"。sage 的 View 是编译产物，交给任何外部代码前都该是拷贝。 |
| 压缩切点：块不可分、永不在 toolResult 处切 | **抄** | 这正是 sage 的「块 = 一次工具调用 + 它的结果，只能整进整出」，pi 已经用产品验证过它是对的；连 split turn 这种"一个 turn 就超预算"的边界情况也有现成解法可抄。 |
| 「真实 usage 当基数 + 只估尾部」 | **抄** | sage 的成本不等式要算旧视图/新视图的 token，纯 `chars/4` 会把不等式算飞；拿 provider 回来的真数当锚是最便宜的校准。 |
| 阈值触发压缩 | **不抄** | 备忘的核心反对意见：它是被阈值逼出来的，不是被算出来的。sage 用成本不等式决定何时开新阶段。 |
| system prompt 尾部放 `Current date` | **不抄** | 前缀要逐字节稳定；把每天会变的东西钉在 system prompt 末尾，等于给缓存埋一个定时失效。sage 的日期该进阶段头部那条消息。 |
| `cache_control` 三点式打法（system / 最后一个 tool / 最后一条 user 消息） | **改造后抄** | 打法本身是对的（provider 只认前缀，breakpoint 往尾部滚）。但在 pi 里它是 provider 自己的行为、harness 完全不知情；sage 的阶段冻结要求**编译器知道断点在哪**，所以这层要抬到 View 与 provider 的接口上。 |
| `transformMessages` 修孤儿 toolCall / 丢弃 error 消息 | **改造后抄** | 「视图必须成对」这件事 pi 用**修复**解决，备忘用**断言**解决。两者不冲突：sage 应在 seam 上断言（编译器的 bug 要当场暴露），在 provider 适配层保留修复（历史里本来就有崩溃留下的半截记录）。 |

---

## 3. 工具暴露

**机制**

- **全量暴露，无延迟、无搜索**：一轮的工具集就是 `activeToolNames` 过滤后的 `TTool[]`，随 turn snapshot 一起冻结并整份发给 provider（`packages/agent/src/harness/agent-harness.ts:182, 318-354`）。全仓搜不到 tool search / deferred tools。
- 运行时可换工具集：`setActiveTools(names)` / `setTools(tools, activeNames?)`，非法名直接 reject（`agent-harness.ts:875-930`）；扩展侧暴露成 `pi.setActiveTools()`（`docs/extensions.md:1483`）。
- **工具结果截断是双限位**：行数 2000 或字节 50KB，先撞到哪个算哪个，且**不返回半行**（`packages/coding-agent/src/core/tools/truncate.ts:1-12`）。
- **截断留下的是"留桩"而不是"占位符"**——每条都自带取回方式：
  - `read`：`[Showing lines 1-2000 of 8123. Use offset=2001 to continue.]`（`packages/coding-agent/src/core/tools/read.ts:315-317`）；单行就超限时改推 bash 兜底 `[Line N is 120KB, exceeds 50KB limit. Use bash: sed -n 'Np' <path> | head -c 51200]`（`read.ts:305-307`）。
  - `bash`：全量输出**溢写到临时文件**，结果里写 `[Showing lines X-Y of N. Full output: <path>]`（`packages/coding-agent/src/core/tools/bash.ts:353-366`）。
- **skills 本身就是两档**：system prompt 里只挂 `<name>/<description>/<location>` 的目录，正文让模型用 `read` 工具按需取（`packages/coding-agent/src/core/skills.ts:336-361`）。
- 做摘要时工具结果另有一道 2000 字符截断，并注明截掉了多少字符（`docs/compaction.md:267`）。

**对 sage 的判断**

| 项 | 判断 | 理由 |
|---|---|---|
| 留桩自带取回指令（`offset=`、`Full output: <path>`、`sed -n` 兜底） | **抄** | 备忘区分留桩与占位符的那一整段，pi 已经在**工具结果层**做成了产品。sage 把它抬到**视图层**（`expand("c0142")`）即可，句式直接沿用。 |
| bash 全量输出溢写临时文件 | **抄** | 这是「原文还在，按编号来取」的最便宜实现：bash 输出不可重现，只有落盘才谈得上 expand。 |
| skills 目录 = 一行档，正文 = 全文档，按需升档 | **抄** | 现成的两档实证，且证明了模型能照着 location 自己去取。sage 的四档模型（不展示/一行/摘要/全文）可以拿它当第一个 claim 的样板。 |
| 双限位（行 + 字节）且不返回半行 | **抄** | 便宜、确定、可复现；比"估算 token 后切"更适合做编译器第 ② 步的确定性淘汰。 |
| 工具全量暴露 + `setActiveTools` 粗粒度开关 | **改造后抄** | 「一轮发哪些工具」是编译器该管的事。sage 里工具目录本身就是一张 claim：预算紧就整体降到一行档，模型仍知道有什么可用、只是不知道参数长相（备忘原话）。`setActiveTools` 这种二值开关不够用。 |
| 没有 expand / 重取的**视图级**机制 | **不抄（引以为戒）** | 目前只有"原文被工具截了"这一种留桩；"编译器主动省略一整块"根本不存在，因此也没有 expand 率这个指标。sage 的 expand 率是打分器的唯一在线体检，第一天就得有。 |

---

## 4. 子 agent 与并行

**机制**

- 子 agent 是**扩展**，不是内核能力：`examples/extensions/subagent/index.ts` 直接 `spawn` 一个新的 `pi` 进程，参数 `--mode json -p --no-session`，再按 agent 定义追加 `--model` / `--tools` / `--append-system-prompt <临时文件>`，最后把任务当 prompt 传进去（`subagent/index.ts:288-330`）。
- **子任务拿到的上下文 = 零**：只有一句 `Task: <task>` 加上 agent 自己的 system prompt。父会话的历史一条都不传。
- 结果通过子进程 stdout 的 JSONL 事件流回来（`message_end` 等），父侧边跑边把最终文本渲染出来，并累计 `input/output/cacheRead/cacheWrite/cost/turns`（`subagent/index.ts:329-400`、README「Isolated context / Streaming output / Usage tracking」）。
- 三种模式 single / parallel / chain，`chain` 用 `{previous}` 占位符串联，parallel 有 `MAX_PARALLEL_TASKS` 上限（`subagent/index.ts:158, 445-460, 583`）。
- agent 定义是带 frontmatter 的 `.md`，`~/.pi/agent/agents/` 与项目 `.pi/agents/` 两级发现（`subagent/agents.ts:26-60`）；README 专门有一节 Security Model 讲项目级 agent 是"仓库可控的提示词"。
- **只读/写分离没有内核支持**：`plan-mode` 扩展靠"白名单工具名 + bash 命令白名单"实现只读探索（`examples/extensions/plan-mode/index.ts:22-23` 的 `PLAN_MODE_TOOLS` 与 `isSafeCommand`）。工具定义上搜不到 `readOnly` / `dangerous` 一类元数据。
- 工具级并行在内核里：按工具的 `executionMode`，只要有一个声明 `sequential` 就整批串行，否则并行（`packages/agent/src/agent-loop.ts:366-385`）；事件顺序有明确契约（start 按 assistant 源顺序、end 按完成顺序、最终 toolResult 消息仍按源顺序，`docs/extensions.md:575-585`）。
- **值得单独点名**：`todo` 扩展的状态存在**工具结果的 `details` 里**，而不是外部文件——「这样分支时 todo 状态自动就是那个历史点上正确的状态」（`examples/extensions/todo.ts:1-11`）。

**对 sage 的判断**

| 项 | 判断 | 理由 |
|---|---|---|
| 子 agent = 另起一个进程、上下文从零 | **不抄** | 备忘的「三样别人做不到的」第三条就是反这个：子 agent 应是换个问题、换个预算，在**同一份 Log 上再编译一次**。pi 的做法是"从零起步 vs 复制一整段"的二选一里选了前者，检索结果无法共享。 |
| 插件状态存在工具结果的 `details` 里，随 Log 走 | **抄** | 直击 sage 的要害：进度账本、claim 的历史、expand 记录都不能放 sidecar，否则一分支就对不上。状态从 Log 派生 = 换个视图编译一次就自动正确。 |
| 子 agent 定义 = 带 frontmatter 的 `.md`，用户级 + 项目级两层发现 | **抄** | 便宜、可 diff、可进仓库评审；sage 的子编译预设（问题 + 预算 + 工具集）用同一形态即可。 |
| 子任务结果按 JSONL 事件流增量回灌，并带全套 usage | **改造后抄** | 流式与计费口径可以照搬；但 sage 回来的不该只有一段文本，而是**块编号**——父视图里能引用同一份原文，这正是"块编号在三份视图里指的是同一份原文"。 |
| 只读靠扩展维护工具名白名单 | **不抄** | 白名单会漂。危险度该是工具定义上的元数据，由编译器/内核读；后台只读任务（审查、eval、跨模型复看）要靠它才能安全共享同一份 Log。 |
| 工具批内并行 + 一个 `sequential` 就整批串行 | **抄** | 规则简单、可证明、事件顺序有契约。sage 一块就是"调用 + 结果"，这套顺序契约保证块永远成对生成。 |

---

## 5. provider 层

**机制**

- `packages/ai` 是独立包，20+ provider（OpenAI / Anthropic / Google / Vertex / Bedrock / Mistral / Groq / Cerebras / Cloudflare / xAI / OpenRouter / Copilot / Kimi / MiniMax / 任意 OpenAI 兼容端点…），并且**只收支持 function calling 的模型**（`packages/ai/README.md:5, 51-77`）。
- **类型面是 pi 自己的**，不是任何 SDK 的：`UserMessage` / `AssistantMessage` / `ToolResultMessage` + `TextContent` / `ThinkingContent` / `ToolCall` / `ImageContent`（`packages/ai/src/types.ts:203-300`）。provider 只做"pi 类型 ↔ wire 格式"的翻译。
- `AssistantMessage` 自带溯源：`api` / `provider` / `model` / `responseModel`（实际路由到的模型）/ `responseId` / `diagnostics` / `usage` / `stopReason`（`types.ts:276-289`）。
- **usage 带缓存四分账 + 每档独立成本**：`{input, output, cacheRead, cacheWrite, totalTokens, cost:{...}}`（`types.ts:254-268`），成本按 model 定价现算（`packages/ai/src/models.ts:39-44`）。
- **流函数契约写死在类型注释里**：一旦被调用，请求/模型/运行时错误**不许抛**，必须编码进返回的事件流，错误终止要产出一条 `stopReason` 为 `error`/`aborted` 的 `AssistantMessage`（`types.ts:196-209`）。`StopReason` 本身含 `aborted`，`signal` 在 `StreamOptions` 上（`types.ts:87, 150, 269`）。
- 跨 provider 交接由 `transformMessages()` 统一处理：同模型保留 thinking block 与签名，跨模型把 thinking 转成纯文本、丢掉 `thoughtSignature`、按需重写 toolCall id、模型不支持图片就换成占位文字（`transform-messages.ts:60-153`）。
- model 元数据带能力位：`cacheControl` 约定、`supportsLongCacheRetention`、工具定义能否带 `cache_control`（`types.ts:394-432`）。

**对 sage 的判断**

| 项 | 判断 | 理由 |
|---|---|---|
| 自有消息类型面 + provider 只做翻译 | **抄** | Log 里存的必须是 sage 自己的结构（块、档位、claim 都要挂在上面）。绑任何厂商 SDK 的类型，阶段边界换模型就成了幻想。 |
| `usage` 的 cacheRead/cacheWrite 分账 + 按 model 定价现算成本 | **抄** | 成本不等式的读价/写价必须从这里取。备忘自己标了「0.1 与 1.25 是印象值、未核实」——有这套结构就不需要印象值。 |
| `transformMessages` 的跨模型规整（thinking 降级、tool id 重写、图片降级） | **抄** | 「阶段边界零代价换模型」这句话的全部实现难度都在这里。这是 pi 最难被复刻、也最该直接学的一段。 |
| 流函数"调用后不抛、错误进流"的契约 | **抄** | 让"崩掉的一轮"也是 Log 里一条正常 entry（带 `stopReason: error`），而不是异常路径上的空洞。sage 的编译器因此永远面对结构完整的历史。 |
| 每条 assistant 记 `responseModel` / `responseId` / `api` | **抄** | 事后回答"这一轮到底是谁答的"，是阶段边界换模型的必备证据，成本几乎为零。 |
| 缓存断点由 provider 自己决定、harness 不知情 | **不抄** | sage 的前缀冻结要求编译器说了算断点在哪。断点该由 View 给出（阶段边界 = 冻结前缀的末尾），provider 只负责翻译成各家语法。 |

---

## 6. 扩展机制

**机制**

- 扩展 = 从 `~/.pi/agent/extensions/` 与项目 `.pi/` 加载的 TS 模块，默认导出一个拿 `ExtensionAPI` 的函数；能注册工具、命令、快捷键、CLI flag、消息渲染器、甚至整个 provider（`docs/extensions.md:1213-1611`）。
- 生命周期钩子是一张画出来的图：`session_start` → `input` → `before_agent_start` → `agent_start` → 每轮 `turn_start` / **`context`** / `before_provider_request` / `after_provider_response` → 工具四联（`tool_execution_start` / `tool_call`(可拦) / `tool_result`(可改) / `tool_execution_end`）→ `turn_end` → `agent_end`；外加 `session_before_compact` / `session_before_tree` / `session_before_fork` 等可取消的结构性事件（`docs/extensions.md:268-336`）。
- **`session_before_compact` 可以整个接管压缩**：返回 `{cancel:true}` 或自带 `{summary, firstKeptEntryId, tokensBefore, details}`，并且 `details` 随便存什么 JSON（`docs/compaction.md:269-341`）。
- 新一代 hook 设计（`packages/agent/docs/hooks.md`，**设计完成、未实现**）把两件事分开：`observe()` 看全部事件、只读、返回值忽略；`on(type, handler)` 参与该事件语义。harness 只调 `hooks.emit(event)`，**每个事件的归约语义写在一张 reducer 表里**——transform 链式、`tool_call` 命中 block 就早退、`tool_result` 逐个打补丁、`session_before_*` 命中 cancel 就早退（`hooks.md:124-148, 160-300`）。事件的结果类型用 phantom symbol 挂在事件类型上，没有额外的结果映射表（`hooks.md:7-50`）。
- 顺序：始终是**注册/加载顺序**，没有优先级、没有依赖声明（`runner.ts:858-886`、`docs/extensions.md:600-612`）。
- skills 是纯数据资源（`SKILL.md` + frontmatter），与扩展是两套东西（`core/skills.ts`）。

**对 sage 的判断**

| 项 | 判断 | 理由 |
|---|---|---|
| `observe()` 与 `on()` 分家（只读观察 vs 参与语义） | **抄** | 和备忘的"观察不影响执行"同构。sage 里"看清单"的人很多，能改视图的只有编译器——这条边界得在 API 形状上就立住。 |
| 每个事件的归约语义集中在一张 reducer 表，harness 只 `emit` | **抄** | 内核不懂插件策略，这正是"内核只拥有 Log 与 View seam"。sage 的编译器同理：它解分配，内核只知道调用它。 |
| 结构性事件可 cancel / 可整体接管（`session_before_compact`） | **改造后抄** | 「打分器可换件」需要的就是这种接管点，但 pi 的接管粒度是"整个压缩"；sage 该把接管点缩到**第 ③ 步打分器**这一个部件，前后步骤不受影响。 |
| 插件顺序 = 加载顺序，靠注释维持铁律 | **不抄** | 备忘记成 G2 的就是这个。claim 模型下压根没有"顺序"这个自由度：编译器在总预算下解一次分配，谁挤掉谁写在清单上。 |
| 扩展可存任意 `details` 到 Log entry | **抄** | claim 的历史、清单、expand 记录都要能落进 Log 而不污染内核的 schema。留一个不透明的 `details` 字段是最便宜的做法。 |
| 扩展能注册整个 provider | **不抄（至少不在第一天）** | 备忘的"最简实现"取向：sage 的 provider 面由 `packages/ai` 式的注册表承担就够，把它再开放给插件只会让"谁改了发出去的内容"这个问题多一条暗路。 |

---

## 7. 可观测

**机制**

- **每次调用之后留下什么**：一条 `AssistantMessage` entry，含 `api/provider/model/responseModel/responseId/usage/cost/stopReason/diagnostics/timestamp`（`packages/ai/src/types.ts:276-289`），随 Log 落盘。
- **每次调用之前发了什么：不记录。** Log 里没有 system prompt、没有工具清单、没有这一轮真正发出去的消息数组。要回答"模型当时看到了什么"，只能重跑 `buildSessionContext` + 那条 `transformContext`——而扩展链是有副作用、依赖加载顺序与外部文件的，所以**不保证重现**。
- 对外有一条完整的事件流：`pi --mode json` 把全部会话事件按 JSONL 打到 stdout，事件类型在文档里逐条列出（`packages/coding-agent/docs/json.md:1-45`），子 agent 扩展就是靠它回灌结果。
- `packages/agent/docs/observability.md` 是一份**完整但未实现**的设计：runtime 无关的 `traceOperation()` / span 树 / `AsyncLocalStorage` 只作为运行时适配器 / 事件名清单（`pi.agent.prompt`、`pi.ai.provider.request`…）/ **默认安全字段与默认不安全字段的两张白名单**（prompt、tool args、tool result、provider payload 一律默认不采）/ 「观察者绝不能影响执行」。全仓 grep 不到 `traceOperation`——**未实现**。
- 缓存这件事有一个**手写探针**而不是产品能力：`packages/coding-agent/test/sdk-codex-cache-probe-tool-loop.ts:378-431` 跑多轮 append-only 对话，逐子请求打印 usage，并断言 `cacheRead` 单调不降，退化点单独列出来。
- 另有 `/export` 导出 HTML 会话、`PI_TIMING=1` 的启动分段计时（`core/timings.ts:1-30`）。

**对 sage 的判断**

| 项 | 判断 | 理由 |
|---|---|---|
| 「不记录这一轮发了什么」 | **不抄（这正是 sage 的立身之本）** | 备忘那句「不允许有不知道为什么在那里的 token」，在 pi 里是做不到的：没有清单，事后只能靠重跑一条有副作用的插件链去猜。sage 的编译清单第一天就要写进 Log。 |
| `cacheRead` 单调性探针 | **抄** | 现成的"阶段冻结有没有真的成立"的测法，而且它已经证明这个数**测得出来、会退化、值得盯**。sage 把它从手写脚本升级成常驻指标即可。 |
| 事件流 = 公开协议（`--mode json`） | **抄** | 子 agent、后台只读任务、外部 UI 用同一条通道；sage 的清单顺着这条流出去，外面就能实时看见每一块为什么在。 |
| observability 设计里的两张白名单（安全字段 / 不安全字段） | **抄** | 清单会写进 Log 并可能外流，块的 `reason` 与 token 数是安全的、原文不是。这条边界一开始不划，后面没法补。 |
| 观察者错误必须被吞、绝不影响执行；控制面 hook 才允许影响 | **抄** | 直接对应 sage 的"编译器是控制面、清单消费者是观察面"。 |
| 「回放」 | **无（该补）** | pi 没有任何回放控制器：策略换了拿不出对照。备忘第 ④ 期的离线回放是 sage 相对 pi 的真增量，而 Log 是树这件事让回放天然可行——重编译一条路径不需要复制任何东西。 |

---

## 8. 工程手艺

**机制**

- **测试跑在无凭据环境**：`test.sh:1-75` 先把 `~/.pi/agent/auth.json` 挪走，再逐个 `unset` 三十多个 provider 的 API key 环境变量，最后才 `npm test`——保证没有任何测试偷偷依赖真实 provider。
- **faux provider 是一等公民**：`packages/ai/src/providers/faux.ts`（499 行）提供 `registerFauxProvider` / `fauxAssistantMessage`，可以检查传进来的 `StreamOptions`、触发 `onPayload`、返回脚本化的 assistant 消息（`docs/agent-harness.md:233`）。
- **测试分层有明文规矩**：`test/suite/` 只许用新 harness + faux provider、禁真 API/网络/付费 token；issue 级回归放 `test/suite/regressions/`，命名 `<issue-number>-<short-slug>.test.ts`（`packages/coding-agent/test/suite/README.md:1-18`）。harness 覆盖率是独立 target（`npm run coverage:harness`，只统计 `src/harness/**` 加它真正跑到的两个运行时文件，`docs/agent-harness.md:235-242`）。
- **CI 一个 job 三步**：`npm run build` → `npm run check` → `npm test`（`.github/workflows/ci.yml`）。`check` 是复合门：biome（`--error-on-warnings`）+ 依赖是否钉死版本 + TS 相对导入规范 + shrinkwrap 一致性 + `tsgo --noEmit` + **浏览器烟测**（`package.json` scripts）。另有每日 `npm audit --omit=dev` 与签名校验。
- **供应链纪律**：直接外部依赖必须钉精确版本；本地一律 `npm install --ignore-scripts`；带 lifecycle script 的新依赖要进显式 allowlist；pre-commit 拦截误提 lockfile（`AGENTS.md:39-47`）。
- **多 agent 并发改同一个 worktree 的 git 铁律**：只 `git add <具体路径>`，禁 `git add -A` / `git reset --hard` / `git checkout .` / `git clean -fd` / `git stash` / `--no-verify`；冲突只解自己的文件（`AGENTS.md:229-285`）。
- **CHANGELOG 纪律**：每包一份，新条目只进 `[Unreleased]`，已发布版本段落不可变，内部改动挂 issue 链接、外部贡献挂 PR + 作者（`AGENTS.md:109-134`）。
- **文档纪律**：设计文档里"已实现 / 计划中"逐条标注（`agent-harness.md` 通篇 Status: Done / In progress / Planned，末尾还有「Completed implementation todo」归档区，`:244-485`），并且在文末单独有一节承认哪些语义仍是 provisional。
- 代码风格硬规矩若干：禁 `any`、禁内联/动态 import、单调用点的单行 helper 必须内联、**不保留向后兼容除非用户明说**（`AGENTS.md:12-24`）。

**对 sage 的判断**

| 项 | 判断 | 理由 |
|---|---|---|
| 测试前剥光所有凭据 + faux provider | **抄（第一天）** | 编译器的正确性全是确定性逻辑（淘汰、打分、分配、清单），本来就不该碰网络。剥凭据是让"不许偷偷调真模型"从规矩变成事实的唯一办法。 |
| 回归测试按 issue 号命名、单独目录 | **抄** | 便宜。打分器判错、块配对错这类问题一定会反复出现，按编号钉住比写在注释里可靠。 |
| 关键子系统单独的覆盖率 target | **抄** | 编译器是 sage 唯一押注的部件，它的覆盖率不该被 CLI/UI 的行数稀释。 |
| 设计文档逐条标 Done / In progress / Planned + 归档区 | **抄** | 备忘自己有一节「这份设计没核对的地方」，是同一种诚实。sage 第一天就该把"哪条是实现了的、哪条是设计"写在同一份文件里。 |
| `check` 是复合门，一条命令覆盖格式/类型/依赖/构建产物一致性 | **抄** | 一个入口、全量输出、不许 tail——这条让"本地是绿的"这句话有意义。 |
| 多 agent 共享 worktree 的 git 铁律 | **抄** | sage 自己就是给 agent 用的运行时，开发期一定会有多个 agent 同时改；这套规矩现成、零成本。 |
| 「不保留向后兼容」 | **抄** | 备忘第 ① 期就是一次内核契约变更，明说不留旧管子、不做兼容层。这条要先成为仓库政策，变更才做得下去。 |
| 把 `context` 这种核心语义只写在文档和注释里 | **不抄** | 见镜头 2/6：能靠机器守住的约束就别靠人读注释。 |

---

## 最值得搬的三件

1. **只追加的 entry 树 + 先落盘再进内存 + leaf 也是一条 entry**（`session.ts:123-251`、`jsonl-storage.ts:226-258`）——sage 的 Log 一上来就该是这个形状：分支、回退、崩溃恢复、子编译共用一份历史，全部落在同一个原语上，不需要第二套机制。
2. **留桩自带取回方式，原文溢写落盘**（`read.ts:305-317`、`bash.ts:353-366`、`skills.ts:336-361`）——备忘区分"留桩 vs 占位符"和四档模型的那一整套，pi 已经在工具层做成产品并跑通了；sage 把同样的句式和溢写机制抬到视图层就是 `expand`。
3. **自有类型面 + `transformMessages` 跨模型规整 + usage 的缓存四分账**（`ai/src/types.ts:203-300`、`transform-messages.ts:60-218`、`models.ts:39-44`）——"阶段边界零代价换模型"的全部实现难度在这三处；成本不等式的读价写价也从这里取真值，不必再用印象值。

## 最该避开的三件

1. **按加载顺序串行改同一个消息数组的插件链**（`runner.ts:858-886`）——顺序只写在文档里、没人算总账、handler 抛错整条链语义就漂；claim + 一次分配从结构上消掉"顺序"这个自由度。
2. **子 agent 另起进程、上下文从零、结果只回一段文本**（`subagent/index.ts:288-330`）与 **`forkFrom` 整文件复制**（`session-manager.ts:1353-1394`）——检索结果无法共享、分支存储线性放大；sage 的子 agent 是换个问题换个预算在同一份 Log 上再编译一次。
3. **"这一轮发了什么"不落任何记录**（Log 只存响应，见镜头 7）——事后要回答"模型当时看到了什么"只能重跑一条有副作用的插件链，等于不可回答；清单是 sage 最便宜也最值钱的一条约束，不能等到第 ② 期才有。
