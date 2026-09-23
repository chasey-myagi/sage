# 上下文编译器按阶段拆分

## Problem

`compiler.ts` 有 1836 行。`view()` 一个函数 377 行，里面的 `assemble` 154 行，定义在 `return view;` 之后，靠函数提升和闭包读外层变量。`runEntry` 126 行，`compileSubtask` 123 行；`spawn.ts` 的 `subtaskTools` 247 行，schema 字面量和两个工具对象都写在里面。

逻辑判断读字符串：`decision.reason.includes("context-window-guard")` 出现三次，`reason.includes("immediate")` 决定开阶段的理由，`rootReason(...).startsWith("scorer:entry:")` 决定近期槽位。reason 里还会拼上 route 策略和打分器给的文字，理论上能误中。

`0.9` 写了三遍，`300`、`8000`、`2`、`8`、`4` 是裸数字。`chunks.find` 在 map 里调用，O(n²)。变量名 `stay` / `fresh` 要读完实现才懂。文件头 113 行，是按 PR 追加的行为清单；65 个函数没有一个 doc comment。

## Decision

纯重构，行为与输出字节不变。`compiler/` 下按阶段拆文件：

| 文件 | 行数 | 职责 | 最长函数 |
|---|---|---|---|
| `compiler.ts` | 564 | `contextCompiler`、选项与默认值、`compileView` 的阶段编排 | `compileView` 56 |
| `assemble.ts` | 447 | 收集 claim、`assembleView`（原 `assemble`，提到顶层显式传参）、claim 放置、块行 | `placeClaims` 49 |
| `chunk-decisions.ts` | 149 | 阶段冻结下的块决策、近期槽位让出、冻结记账 | `decideChunks` 42 |
| `epoch.ts` | 276 | 投影、剩余轮数、`chooseEpoch`、开阶段与路由、overrun | `chooseEpoch` 52 |
| `score-rebuild.ts` / `score-entry.ts` / `score-shadow.ts` / `score-common.ts` | 182 / 235 / 114 / 74 | 三种打分器调用与共用件 | `runEntryScorer` 51 |
| `subtask-view.ts` | 328 | 子任务重编译 | `subtaskView` 56 |
| `rows.ts` | 109 | manifest 块行类型与构造器，固定键序 | 15 |
| `reasons.ts` | 99 | reason 字符串的拼装与唯一解码器 | 12 |
| `log-read.ts` | 346 | 路径只读查询 | `projectionSeqs` 31 |
| `breakpoints.ts` | 58 | 缓存断点 | 26 |

`subtask/` 拆成 `spawn.ts`（436 行，工具工厂与子会话，最长 `startChild` 52）、`task-input.ts`（输入校验、预设、`TASK_SCHEMA`）、`admission.ts`（冲突判定、配额、有界池）。最长函数从 377 行降到 56 行。

字符串协议换成结构化字段：

- `decide.ts` 新增 `classifyEpoch`，在设置 reason 的同一分支返回 `trigger`（`context-window-guard` / `immediate` / `payback` / `no-rebuild`）；`decideEpoch` 仍只返回决策，输出不变。`EpochChoice` 带 `trigger`、`isGuardOpenedEpoch`、`isFrozenOverWindow`，护栏判断、开阶段理由、overrun 都读这三个字段。
- 从 Log 读回的块决策只有 reason 字符串可用，`reasons.ts` 的 `reasonKind` 是唯一解码点，返回 `recent` / `kept` / `superseded` / `retry` / `budget` / `entry-scorer` / `other`；近期槽位与重建打分的候选都按它分支。
- `tagPricing` 删掉永远为假的 `includes("pricing:fallback")`：它只作用于 `decideEpoch` 刚生成的 reason。

新增具名常量：`CONTEXT_WINDOW_GUARD_RATIO`、`CONTEXT_WINDOW_GUARD_REASON`、`DEFAULT_KEEP_RECENT`、`DEFAULT_EXPECTED_TURNS`、`DEFAULT_ALPHA`、`DEFAULT_MIN_SAVINGS_RATIO`、`DEFAULT_MIN_SCORE_TOKENS`、`DEFAULT_SUBTASK_BUDGET`、`DEFAULT_MAX_DEPTH`、`DEFAULT_MAX_SUBTASKS`、`DEFAULT_MAX_CONCURRENT`、`OWN_CHUNK_PREFIX`（`expand.ts` 与子任务共用）。

`chunks.find` 换成先建首个优先的 `firstById` 索引，与 `find` 取第一个的语义一致。`stay` / `fresh` 改名 `frozenView` / `recompiledView`。

文件头重写为契约，每个 ≤ 25 行。原 `compiler.ts` 文件头的 27 条命题逐条搬到实现它的函数或类型的 JSDoc 上。chunk、tier、claim、stub 在 `Chunk`、`Tier`、`Claim`、`renderStub` 上解释，epoch 在 `EpochDecision` 上解释。`compiler/` 与 `subtask/` 的每个导出符号都有 JSDoc。

## Alternatives considered

不在 `ChunkDecision` 或 `ChunkRow` 上加结构化字段。块行原样写进 manifest，加字段会改 Log 字节；冻结决策从 Log 读回时，也只剩 reason 字符串。所以集中到一个解码器，不再各处 `startsWith`。

不改 `decideEpoch` 的返回形状。`decide.test.ts` 断言它的输出，改形状就要改断言。`classifyEpoch` 另开一个出口，`decideEpoch` 包一层。

`isRecord`、`errorText`、`EMPTY_USAGE` 的重复副本不在这次处理，由统一下沉的 PR 负责。本次只把 `compiler.ts` 里那份移到 `log-read.ts`，把 `spawn.ts` 那份移到 `task-input.ts`，副本数量不变。

## Consequences

结构化判断不再受 reason 文字影响。以前 route 策略或打分器 fallback 的文字里如果含 `context-window-guard` 或 `immediate`，会误触护栏或把开阶段理由写成 `cost:immediate`；现有 route 与 fallback 文字都不含这两个词，所以现有输出不变。

core 的 `BudgetReport` 没有声明 `omitted` 与 `expands`，manifest 却一直写着这两个字段。编译器只能先在字面量之外构造 budget 对象再放进去。补类型属于 core 的 PR。

子任务视图的前缀断点没有测试断言。拆分时曾把它算错（用了追加子消息之后的数组长度），全部测试照样是绿的，是下文的快照比对发现的。

## Testing

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、85 个文件、412 个测试）。测试文件一行没改。

另做了一次字节比对：临时包一层 `contextCompiler` 与两个 spawn 工具，把全部测试里每次 `view()` 的返回值和每次工具结果按用例记成 JSON，重构前后各跑一次。95 个用例、355 条记录，去掉随机 UUID 后逐字节相同。包装代码没有提交。
