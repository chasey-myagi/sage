# Agent Note: jev 问题收窄到规则会丢的块

## Problem

jev 打分器对每个候选块都问同一组四个固定问题加逐行 `keep_<i>`，合成时 `is_edit_target`、`answers_lookup` 能把块判 full，`is_broad_listing` 能把块判 none。离线校准换成「内容后来被用到」的标签后，这三问在 read 块上没有区分力，`is_edit_target` 还是反向的：模型改文件前多半会重读，「将被编辑」预测的其实是「这个旧窗口会作废」。在 bash 运行类和 grep 上有区分力的是 `reports_failure` 和 `keep_<i>`。Jev 该判断的只是规则会丢掉的那些块里哪些要留，不该决定 read 的档位，也不该为 ls、find 之类的输出花 token。

## Decision

- **删掉 `is_edit_target`、`answers_lookup`、`is_broad_listing`**，问题本身和合成依据一起删。
- **问题集按块类型选**，`questionSetFor(chunk)` 在 `jev/questions.ts`，块类型取规则打分器的 `ruleCategory(chunk)`，工具名与 bash 分类（`classifyBashCommand`）只有一份定义：
  - grep，以及判为 `runner` 或 `mixed` 的 bash（含 `isError` 的运行类）：分诊问题集 `triage`，问 `reports_failure`、新增的 `locates_next_step` 和逐行 `keep_<i>`。
  - read：探针问题集 `read-probe`，只问新增的 `shows_target_code`。
  - 其余（ls、find、读取类 bash、参数里没有字符串 `command` 的 bash、其他工具）：没有问题集，不发请求。
- **read 不给决定。**`shows_target_code` 是试验项，不参与合成；线上打分器不发 read 探针（回答无处可用，发了只花钱），只有离线的 `entryJevRequests` 建这个请求，`sage calibrate-jev` 把它的概率写进 `sweep.json`。没有问题集或只有探针的块，线上不给决定，档位由编译器的规则决定。
- **`isError` 只对要分诊的块直判 full。**先选问题集，只有分诊块在 `isError` 时不发请求、直接判 `scorer:jev:full:is_error`；read、ls 等即使 `isError` 也不给决定，重建时不覆盖淘汰阶段的档位。
- **合成规则**（`decideTier` 只接分诊回答，`TriageJudgment`）：`reports_failure` 或 `locates_next_step` 不低于 `full` 判 full；这两项和每个 `keep_*` 都低于 `low` 判 none（没有候选行时只看前两项）；否则从 `keep_*` 不低于 `keep` 的行里摘至多 3 行判 summary，摘不出可定位的笔记就判 full。阈值仍是 `DEFAULT_JEV_THRESHOLDS`，缺省值不变，`thresholds` 选项可覆盖；`none` 档原本只服务于 `is_broad_listing`，随它删去，阈值变成 `{ full, low, keep }`。reason 变成 `scorer:jev:<tier>:failure=…,locate=…[,keep=…]`。
- **回答读取是判别联合。**`readJudgment(answers, set, candidateCount)` 返回 `Judgment = TriageJudgment | ReadProbeJudgment`，`ChunkRequest` 带上 `set`。
- **候选行提取器只留会被问到的工具**：read、grep、bash。ls / find 的条目提取和未知工具的通用提取没有调用方了，一并删掉。`READ_TOOL`、`GREP_TOOL`、`BASH_TOOL` 从 `scorer/rule.ts` 导出，规则表、问题集与候选行提取共用。
- **`calibrate-jev` 跟着变。**扫描只对分诊块定档，网格去掉 `none`、`--t-none` 旗标删除，默认网格从 180 组变成 45 组。`sweep.json` 的 `chunks` 列出每个拿到回答的块（含 read 探针），新增 `tool`、`bashKind`（bash 块的 `classifyBashCommand` 结果，其他为 null）、`questions`（问题集）和 `answers`：分诊块是 `{ reports_failure, locates_next_step, keep }`，read 探针是 `{ shows_target_code }`，都是原始概率，不含文本。`skipped` 新增 `unasked`；`unpriced` 只数进扫描的块。缓存键仍是请求体的 sha256，请求体变了，旧缓存自然不再命中。

## Alternatives considered

- **线上也发 read 探针，把概率写进 reason**：read 不给决定，reason 只挂在决定上；要记录就得给 `ScoreResult` 加一个只审计的字段并写进清单，这是 Log 形状的改动，且每个 read 块都多一次付费请求换不来任何档位变化。探针先只在离线跑，有区分力再谈接入合成。
- **read 也给 full 决定来挂 reason**：入口模式下 full 决定虽然不动原文，但在重建模式里会覆盖规则的结论，等于让 Jev 决定 read 的档位，正是要去掉的东西。
- **保留 `none` 阈值，改成「`keep` 的最大值低于 `none`」**：同一个「都很低」的判断用两个阈值表达，校准网格还要多扫一维。issue 的合成规则只说「都很低」，一个 `low` 足够。
- **给 `jevScorer` / `entryJevRequests` 留可注入的 `classifyBash` 选项**：`ruleCategory` 已经调用 `classifyBashCommand`，再留一个注入点就有两个 bash 分类来源，同一个块可能被规则与 jev 判成不同类型。
- **保留 ls / find 的候选提取**：没有任何请求会用到，只剩测试在钉它。

## Consequences

- read、ls、find 块不再有 jev 决定，编译器保留它们原来的档位（入口模式下仍是 full，重建时是淘汰阶段的档位）；之前 jev 会把宽泛列表判 none，现在不会。
- jev 的 bash 分类跟着 `classifyBashCommand` 走：它把命令判错时，jev 的发问范围一起错。
- 旧的校准缓存不再命中，重跑会重新计费。
- `shows_target_code` 的概率只出现在 `sweep.json`，要它参与合成需要另开一件事，并决定 read 由 Jev 还是规则说了算。
- 包入口新增导出 `TriageJudgment`；`EntryJevRequest` 新增 `bashKind`。

## Testing

- `packages/plugins/test/scorer/jev.test.ts`：分诊问题集的题目与措辞逐字钉住（`reports_failure`、`locates_next_step`、`keep_<i>`）；`chunkRequest` 对 read 只出 `shows_target_code`；read、ls、未知工具不发请求也不给决定；由真实的 `classifyBashCommand` 判为读取类的 bash 不发，运行类与混合类发；参数里没有字符串 `command` 的 bash 不发；`isError` 的 read、ls 不给决定，`isError` 的运行类 bash 判 full。合成分支各一例：`reports_failure` 到 `full` 判 full、`locates_next_step` 到 `full` 判 full、全部低于 `low` 判 none（任一项到 `low` 就不是 none）、没有候选行时只看两项、summary 摘 3 行、有行过 `low` 但没有行到 `keep` 判 full、笔记不可定位判 full、阈值从选项覆盖。
- `jev-size.test.ts`：典型 grep 与 vitest 分诊请求不超过 26 题、20KB；300 行 TS 文件的 read 探针 1 题、20KB 内。
- `jev-excerpt.test.ts`、`jev-calls.test.ts`、`test/regressions/issue-181.test.ts` 的块换成 grep，原有的部分失败、熔断、入口与重建路径照旧覆盖。
- `apps/sage/test/calibrate-jev.test.ts`：录制会话改为四次 read、两次 grep、一次运行类 bash 加一次 `expand`；校准发出的分诊请求体与线上入口打分逐个相等，read 另外各发一个只有 `shows_target_code` 的探针；`sweep.json` 每块带 `tool`、`bashKind`、`questions`、`answers`；被 `expand` 的 grep 标为需要；扫描只在分诊块上定档，平局推荐离默认值最近的一组。
- `apps/sage/test/jev-endpoint.test.ts` 的会话改为 grep，因为 read 不再发请求。
- 测试不打真网，Jev 只经 `jsonFetch` 替身。
- `pnpm check` 全绿。
