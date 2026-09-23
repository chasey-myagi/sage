# 参考仓取材：抄什么、不抄什么、为什么

> 状态：设计输入，2026-09-22。三份逐仓侦察原文在 `recon/`（deepseek-harness / pi-mono / kimi-code），每条判断都带 `file:line`。本文只写综合后的结论；结论与总纲 #1 冲突时以本文为准并回改 #1。
> 核心不变：sage 的观点来自那篇备忘（上下文是从只追加的 Log 编译出来的视图，内核只有 Log 与 View seam）。参考仓只提供「已被验证过的做法」和「已经付过学费的坑」。

## 0. 三个仓一句话

| 仓 | 规模 | 与备忘的关系 | 对 sage 的价值 |
|---|---|---|---|
| deepseek-harness (`dsh`) | 30 万行源码，247 包 | **原语层已做到**：Session 是只追加事件日志，消息由 `deriveMessages()` 投影；缺的是备忘后半段（预算、打分、清单） | 原语层对照抄；工程纪律最成熟（Model Experience 三问、Agent Note、keyless 回放） |
| pi-mono (`pi`) | 8 万行源码 | harness-pi 的上一代：Log 是只追加的树，视图每轮由一条 `transformContext` 重算；无预算无清单 | provider 层最值得学（跨模型规整、usage 分账、错误进流）；工具层留桩已是产品 |
| kimi-code (`agent-core-v2`) | 9 万行源码 | Log 是 Op 流 + 纯 reducer，状态全由重放算出；已在同一份 Log 上编译出「给人 / 给模型」两份视图；仍是阈值压缩 + 复制式 fork | Log 的形状（Op + reducer + 内容寻址清单）与测试形态（前缀感知快照）最直接可用 |

三家共同的病，正是备忘要治的：**压缩由阈值触发、一刀切有损；子 agent 要么从零起步要么复制整段日志；没有人记录「这一轮为什么发了这些」。** 三家都没有：预算申领、档位、成本不等式、离线回放。这四样是 sage 相对它们的真增量。

## 1. 契约修订（进 #1 总纲「契约条款」一节）

编号前缀：C = deepseek-harness，P = pi-mono，K = kimi-code。每条注明落到哪张票。

### Log
- **L1 · Log 是只追加的树，先落盘再进内存**（P5, K1）：entry 带 `id / parentId / seq`，leaf 也是一条 entry；append 先写 JSONL 再进内存，磁盘永远是内存的超集；读入时尾部撕裂的最后一行丢弃、其余位置损坏抛错。不再有 HWM 计数器。→ #4
- **L2 · entry 是带类型的记录，reducer 在内核外**（K1, C7）：内核只会 append / replay / leaf；每种 entry 类型在 import 期注册自己的纯 reducer；读到未注册类型**拒绝重建**（fail-closed 词表），不跳过。类型清单由脚本生成并有新鲜度检查（K7）。→ #4, #2
- **L3 · 改写也是追加**（K1）：撤销、压缩、换模型、开新阶段都是新 entry（`view_epoch` / `model_change` / `undo`），原记录永远在。→ #4
- **L4 · model-visible ⟺ logged**（C4）：任何进入模型请求的内容都必须能从 Log 重建；新增一种模型可见输入 = 新增一种 entry。请求信封（provider / model / 采样参数 / `systemPromptHash` / `toolsHash` / adapter 补的默认值）作为 `request_header` entry 落 Log，**内容寻址**：system prompt 与 tool schema 都按 hash 存，只有首见或与上一条 `request_header` 不同时才存全文（K5；原稿写「system prompt 只在偏离默认时存全文」，实现时没有「默认 prompt」这个概念，2026-09-24 按实现改写）。→ #4, #8
- **L5 · 三域边界**（C5）：可重放的事实进 Log（纯 JSON，无活对象）；活的拦截与瞬时信号是运行时事件；同一事实不许两个来源。→ #5, AGENTS.md
- **L6 · 临时注入不进 Log**（K12 反例）：提醒、协议消息、子任务说明都是视图里的 claim 产物，不是历史。kimi 为此付了三处特判的利息。→ #8, #11
- **L7 · entry 留不透明 `details`**（P6）：claim 历史、清单、expand 记录落 Log 而不污染内核 schema；插件状态从 Log 派生，不放 sidecar。→ #4
- **L8 · 每条 assistant 记 `responseModel / responseId / api / traceId`**（P4, K7）：换模型与对 provider 日志的证据链。→ #4

### View seam
- **V1 · 头随阶段冻结**（C1）：请求 = 头（system + tools schema）+ 消息；头在所有 provider 线格式里都排在消息前。阶段冻结的是**头 + 前缀消息**；工具激活、persona、常驻片段进头，只在阶段边界生效。→ #5, #12, #11
- **V2 · 缓存断点由 View 给出，provider 只翻译**（P1）：`CompiledView.cacheBreakpoints` 标在冻结前缀末尾；Anthropic 适配翻成 `cache_control`，OpenAI 兼容侧无操作。断点不许由 provider 自作主张。→ #3, #5
- **V3 · seam 断言配对，投影层修复并留痕**（C6, K2, pi）：内核在 seam 上断言 toolCall/toolResult 成对，不成对当场抛（编译器的 bug 要暴露）；投影层对**崩溃留下的**半截记录做修复并把「修了什么」写进清单 reason；悬空 toolCall 合成结果分两种语义——`TOOL_NOT_STARTED`（可重试）/ `TOOL_OUTCOME_UNKNOWN`（先验外部状态，不盲重试），文本写给模型看。→ #5, #4
- **V4 · View 交出前是拷贝**（P8）：插件永远改不到 Log。→ #5
- **V5 · `observe()` 与 `on()` 分家**（P7）：观察者错误被吞、绝不影响执行；能改视图的只有编译器。→ #5
- **V6 · 权限形状代替顺序**（C10）：工具管线固定为 pre-execute（可 allow/deny/ask）→ guard（只能否决，不能放行）→ execute（around）→ post-execute（可改结果）→ result（只读通知）；顺序不再是正确性问题。少数确需链的位置用「声明化顺序 + 目标缺失即抛」（K6）。→ #5, #6

### 编译器与 claim
- **E1 · 块边界是一个谓词，不是策略私有函数**（K2 `canSplitAfter`, C9）：user 消息之后不切、带 toolCalls 的 assistant 之后不切、未闭合的 tool 交换不切。淘汰与打分都只在合法边界动手，内核断言不替策略兜底。→ #9
- **E2 · 动态内容只在变了才追加**（C8）：pinned 的进度账本内容没变就不写新条目。→ #11
- **E3 · 摘要档自带前言**（K9）：「这是你自己的笔记，不是证据」；用户原话逐字保留（超长取头尾 + elision 标记）。→ #11, #15
- **E4 · 留桩句式与落盘**（P12, K3, C14）：`[#id · tool · 摘要 · n tokens · 已省略 · expand("id")]`；bash 全量输出与超大结果落盘，头尾预览 + locator + 取回提示，**通知行的字节预留在上限内**；双限位（行 + 字节）不返回半行。→ #10
- **E5 · 工具目录「一行档」已被验证**（K3 名字增量公告 / P12 skills 目录）：sage 用 claim 的 `tiers.line / full` 表达；但激活工具改 tools 块 = 砸缓存，所以**激活只在阶段边界生效**（V1）。→ #11
- **E6 · 真实 usage 当基数、只估尾部**（P9, K10）：成本不等式的旧/新视图 token 用 provider 回来的真数锚定；`context_size.measured` 记真值 + 条数，尾部才估算。→ #12
- **E7 · 辅助调用只许是主请求的前缀延长**（C3）：打分器、摘要器、路由判断若用模型，逐字节复用主请求的头与历史，指令追加在尾部，用不到的 tools 也照带。→ #15, #13
  - 推论（#15 落地时确认）：缓存按型号隔离，所以辅助调用必须用**同一型号**才吃得到前缀缓存（读价 0.1×）；备忘里「换便宜模型打分」与 E7 冲突，E7 赢——大前缀下同型号缓存命中比便宜模型全价更省。sage 的 `modelScorer` 默认即用当前会话型号。
- **E8 · 多投影变体 = 预算更紧的重编译**（K2）：413 / 媒体格式被拒等 provider 拒绝，用「换参数再编译一次」处理，不写特判函数。→ #8

### 子 agent
- **S1 · 子视图与主视图共用同一个头**（C2）：差异全部落在消息尾部；子专属工具（report）做不到 schema 与父同就不加，改成父子共享 tools 块 + 尾部文本说明。→ #14
- **S2 · 只读并发的准入靠资源声明**（K4 `ToolAccesses`）：工具声明 `accesses: {key, mode: read|write|search, recursive}`，冲突谓词决定能否并发；这同时是 #9 确定性淘汰的 `resourceKeys` 来源——一份声明两个用途。→ #6, #14
- **S3 · 子任务回灌带块编号**（P13）：不是纯文本摘要；父视图能引用同一份原文。子 agent 预设 = 带 frontmatter 的 .md，用户级 + 项目级两层发现。→ #14
- **S4 · 不抄**：fork 复制事件/日志（C, K）；子 agent 另起进程从零（P, K）。

### provider
- **R1 · contract 层零 SDK 依赖，脚本强制**（K5, P）：`packages/provider/contract` 只有纯类型与纯函数；CI 脚本查 import。→ #3, #2
- **R2 · 双适配器当契约孪生**（C11）：Anthropic + OpenAI 兼容同时打同一份 `StreamChunk` 词表；「词表表达不了其中任何一个，就是词表的 bug」。usage 必须早于 finish；错误只有两条路径（抛 / `finish{error|aborted}`）；`adapterDefaults` 标出适配器补的值。→ #3
- **R3 · 流不抛错，错误进流**（P3）：崩掉的一轮也是 Log 里一条正常 entry。→ #3, #4
- **R4 · usage 四元组 + raw 逃生口**（K5）：`inputOther / inputCacheRead / inputCacheCreation / output`，各家字段在适配器抹平；`FinishReason` 归一化为固定枚举 + `rawFinishReason`。→ #3
- **R5 · 跨模型规整是换模型的真实成本**（P2）：thinking 块降级、tool id 重写、图片降级；#13 必须包含这一层。→ #13
- **R6 · `prepareCall` 两段式**（C11）：先解析模型拿到 `contextWindow` 与定价，再绑定 dispatch；解析结果参与成本不等式。→ #3, #12

### 可观测与测试
- **O1 · 无 key 回放是默认测试档**（C12, P11）：录制的会话 JSONL 既是输入也是 oracle；测试前剥光凭据 + faux provider；`record` 才打真 API。→ #7
- **O2 · 前缀感知的 LLM 输入快照**（K6）：把一次调用渲染成 `system: / tools: / messages:`；历史是上次的前缀时只打印 `<last>` + 增量。任何破坏前缀稳定的改动当场炸快照——「阶段冻结成立了吗」从指标降级为默认断言。→ #7
- **O3 · 期望产物提交进仓库**（C12）：`system-prompt.expected.md` / `tool-schemas.expected.json` / 编译清单，PR diff 逐行看。→ #7
- **O4 · cacheRead 单调性探针**升级为常驻指标（P10）。→ #17
- **O5 · 清单外流的两张白名单**（P）：块的 reason 与 token 数是安全字段，原文不是。→ #8, #17
- **O6 · 同一份 Log 编译「给人的」视图**（K8）：UI/transcript 用另一套 reducer，压缩保留全量历史只加摘要标记。M4 之后的事，先记下。

### 工程纪律（第一天就用）
- **G1 · README 的 Model Experience 三问**（C13）：每个包的 README 必答「模型看到什么 / token 影响 / KV cache 影响」，豁免要写理由入表，CI 门。→ #2
- **G2 · Agent Note 四段式**（C13）：Problem / Decision / Alternatives considered / Consequences，非平凡 PR 必带；归档即冻结。→ #2, AGENTS.md
- **G3 · 分层与纯度用脚本 + 单测守**（K7, C）：core 不许 import 编译器相关模块；plugin 之间不 import；contract 不 import 任何外部包。→ #2
- **G4 · 预发布不留兼容层**（C13, P11）：首个 tag 之前只有一种形状。→ AGENTS.md
- **G5 · 回归测试按 issue 号命名；编译器子系统单独覆盖率目标；`check` 复合门一个入口**（P11）。→ #2, #7
- **G6 · 不可信文本转义**（K11）：注入进视图的外部文本一律转义。→ #11
- **G7 · 文件头即契约，含「故意不进 Log 的东西」**（K8）。→ AGENTS.md

## 2. 明确不抄（与「抄」同权重）

| 不抄 | 谁家的 | 为什么 |
|---|---|---|
| 阈值触发、一刀切有损摘要的压缩 | 三家都有 | 备忘的病灶：这一刀划不划算和阈值无关；sage 由成本不等式批准阶段边界 |
| 服务端阈值压缩（Anthropic Context Editing API） | Claude Code | 官方实现的「服务端清旧块」路线：不可审计、无清单、无法取回；清理点后缓存照样失效。sage 刻意不走 |
| fork = 复制日志 / 事件 | dsh, kimi, pi `forkFrom` | 裁决①算过的放大；sage 的子 agent 是同一份 Log 再编译 |
| 子 agent 另起进程、上下文从零、只回摘要 | pi, kimi | 检索不共享，块编号两边指不到同一份原文 |
| 临时注入写进 Log | kimi | 撤销/退出/协议三处特判的利息 |
| 插件按加载顺序串行改同一个消息数组 | pi, harness-pi | G2；claim 模型下没有「顺序」这个自由度 |
| 缓存断点由 provider 自己决定 | pi, kimi | 前缀冻结要求编译器说了算 |
| system prompt 尾部放日期等易变事实 | pi | 给缓存埋定时失效；进阶段头部消息 |
| 催促 / 三级提醒 / 强停 | kimi | 这份设计只管这一轮发什么 |
| 运行时动态定义插件 | dsh | 与回放冲突：回放时插件已不存在 |
| 双引擎长期并存、五道迁移 | kimi | sage 从零，第一天只留一种形状 |
| 247 包 / 179 脚本级别的分包 | dsh | 两个原语撑不起；过早拆分让「内核小」变反话 |
| 16+ provider 的广度 | pi, kimi | 第一天两类够；但契约形状一次到位 |

## 3. 这份综合没核对的地方
- 三份侦察都是只读、未跑任何命令；`file:line` 基于各自 checkout 的快照。
- pi-mono 本地停在 2026-05-21，备忘点名的 microcompact / deferredTools 等在该快照里不存在（那是 harness-pi 的东西）。
- kimi 的「缓存断点跟着尾巴走、前缀无人保护」是读代码得出的判断，未做运行时验证。
- ~~Claude Code 的 `cache_editing_` 请求头未核实~~ **已核实（2026-09-22，见 #12 评论）**：真实机制是 Anthropic 的 Context Editing API（`anthropic-beta: context-management-2025-06-27`，服务端清旧 tool 结果 / thinking 块）。官方文档明说清理点之后缓存前缀失效、要付一次写入，所以它不能让阶段内「不重算前缀就降档」；#12 的前提不变，不加档位。它的 `clear_at_least` 参数（清得够多才值得让缓存失效）是本设计成本不等式的朴素版。
