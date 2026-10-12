# Agent Note: 规则打分器按工具类型分档，bash 命令分读取类与运行类

## Problem

`ruleScorer` 是个空打分器：每个候选都判 full，带 `isNoop: true`，编译器见到它就跳过改写，入口模式下甚至不调用。`--scorer rule` 因此和不写 `--scorer` 没有区别，CLI 还专门拒绝 `--scorer rule --scorer-apply entry`。可是单凭工具类型就有分档依据：read 和 grep 的结果常被引用；bash 要看命令，读取类（`cat`、`sed -n`、`grep`、`ls`）和 read 相近，运行类（测试、构建、脚本）以退出码 0 结束后输出多半用不到，退出码非 0 时报错正是下一步要看的。这些判断不需要发任何请求。

## Decision

- **`classifyBashCommand(command)` 放在 `packages/plugins/src/scorer/bash-kind.ts`，从包入口导出**，返回 `BashKind = "reader" | "runner" | "mixed"`。纯函数，只看文本。先丢掉 heredoc 正文，再一次扫描、记着引号状态切出部分（`&&`、`||`、`;`、换行）、管道级（`|`）和词。每个词同时保留去引号的 `text` 和把引号内字符换成占位符的 `bare`；运算符、`#` 注释、`>` 重定向都只在 `bare` 上识别，所以 `grep -rn 'x >= 3' src` 是读取类。
- **每级的判断**：去掉开头的括号和 `NAME=value`，跳过复合命令的续接词（`do`、`then`、`fi`……）和前缀包装命令（`sudo`、`env`、`time`、`nice`、`xargs`、`command`、`exec` 及其选项），看剩下的第一个词。`for`、`if`、`while`、`until`、`case`、`select`、`function` 开头判混合。`cd`、`echo` 这类中性命令整级丢掉，这就是「去掉 `cd … &&` 前缀」的一般形式。读取类表、中性表、包装表、git 读取子命令（含 `branch` / `tag` / `remote` 的列举形式与 `worktree list`、`config --get`、`stash list`）、`find` 动作旗标、`sed -i`、`sort -o` 都是带 JSDoc 的具名常量。不认识的命令按运行类；引号外写文件的输出重定向也是运行类，`2>`、`2>&1`、`>&2`、`/dev/null` 不算。
- **管道**：后面的读取类级只是过滤（`cat a | grep x` 是读取类）。bash 工具用 `sh -c` 执行，不开 `pipefail`，管道的退出码是最后一级的，所以第一级是运行类、后面还有级时判为混合；命令里出现 `pipefail` 时仍按运行类。
- **`ruleScorer` 改成工厂 `ruleScorer({ tiers? })`**。候选先由 `ruleCategory(chunk)` 归入七类之一（`echo`、`reader`、`bash-reader`、`bash-runner`、`bash-runner-error`、`bash-mixed`、`other`），再查 `DEFAULT_RULE_TIERS`：只有 bash-runner 判 summary，其余是 `keep`。`keep` 不给判定，块留在编译器原有规则的档位和 reason 上。给了判定的 reason 是 `scorer:rule:<类别>`；入口模式下照 `entryReason` 的现有组合写成 `scorer:entry:<tier>;scorer:rule:<类别>`。
- **echo 类保留、类别名留着**：edit、write、update_ledger 的真实结果只有一行（`Edited <path>`、`Wrote <path>`、`Ledger updated.`）。`none` 只替换 tool_result，桩行要印出调用参数和 token 数，比原结果还长，还会让模型以为省掉了内容。类别留着，`tiers` 可以覆盖，标签工作也能区分它。
- **`ruleCategory` 从包入口导出**，参数是 `Pick<Chunk, "toolName" | "input" | "isError">`，jev 问题收窄等调用方复用同一份工具名映射。`update_ledger` 取自同包 `progressLedger` 的 `LEDGER_TOOL_NAME`；`edit`、`write`、`read`、`grep`、`bash` 对应 `@sage/tools` 的 `TOOL_NAMES`，plugins 不许 import tools，只能写成字面量。bash 的常量叫 `BASH_TOOL`，不叫 `*_TOOL_NAME`，免得被 check-layers 当成插件工具名采集。
- **summary 不生成文本**：`ScoreDecision` 新增 `stubOnly?: true`。入口打分见到它就写成 summary 行、笔记为空串、不做可定位检查；重建打分同样写空笔记。`renderSummary` 遇到空笔记只输出桩行，所以视图、冻结后的下一轮和 `rebuildFromLog` 重建出来的都是同一行桩。
- **`isNoop` 删掉，换成 `isLocal?: true`**：含义是「只看候选、从不发请求」。它现在只有一个用途：离线回放保留这样的打分器，其余的换成缺省档位的 `ruleScorer()`，报告照旧写 `scorer:offline-rule`。
- **`contextCompiler` 的 `scorer` 默认不装**。原来的默认是空的 `ruleScorer`，「没装打分器时与只跑淘汰逐字节相同」靠的就是它；现在规则打分器真会改档，默认值改成 undefined，重建和入口两条路径在没有打分器时直接跳过。CLI 不写 `--scorer` 的行为不变。
- **入口打分成功时的 `calls` 走 `requestCount`**：规则打分器返回 `requested: false`，`entryScored.calls`、`shadow.calls` 都记 0，重建路径没有 usage 也没有 fallback，不写 `manifest.scorer`。`ScoreResult.requested` 的 JSDoc 从「发请求前就放弃」放宽为「没发请求」。
- **CLI**：`--scorer-apply entry` 现在只要求写了 `--scorer`，任何一个都行；报错改为点名 `--scorer rule|model|jev`。

## Alternatives considered

- **echo 判 none**：初版就是这样。桩行比一行回显长，模型看到的反而更多，还暗示有被省掉的输出。
- **summary 的笔记写成命令本身或输出末几行**：issue 要求不额外生成文本；命令已经在桩行里，末几行是另一种摘要策略，属于 jev 那条线。
- **空串笔记在入口打分里一律当成只显示桩行**：不需要新字段，但会改掉模型打分器「空串笔记被拒、保持 full」的既有行为（`issue-87` 回归钉着它）。用显式的 `stubOnly` 只放行明确声明的判定。
- **runner 直接判 none**：桩行相同，但 issue 要的是 summary 档，后续标签工作要区分「运行输出摘要」和别的判定。
- **拆词之后再按正则找重定向**：初版这样做，引号信息在拆词时已经丢了，`awk '$1 > 3'` 会被当成写文件。改成在扫描时就给每个词带上 `bare`。
- **管道里的运行类按运行类判**：退出码被最后一级吃掉，失败的测试也会被摘成桩行，正好丢掉最需要的报错。
- **保留 `isNoop`、让编译器默认仍装规则打分器**：默认编译会悄悄开始改档，所有不写 `--scorer` 的会话与历史清单都会变；默认改成不装打分器，变化只落在显式选了 `rule` 的地方。
- **离线回放把调用型打分器换成「不装」**：标签 `scorer:offline-rule` 就不再属实，要另起一个标签；换成真正的规则打分器，标签字面上仍对，回放结果也更接近一个会改档的策略。

## Consequences

- `--scorer rule` 会改变模型看到的消息：退出码为 0、没接管道（或开了 `pipefail`）的运行类 bash 只剩桩行。入口模式下这些块从第一次进入视图起就是最终形态；重建模式下只在阶段重开时生效。
- 任何打分器给出的空笔记 summary 现在都只显示桩行，不再留下一个后面没有内容的 `summary (model notes, not evidence): ` 标签。这改的是重建模式下模型打分器漏写笔记时的视图。
- 离线回放 jev / model 策略时，档位由规则打分器给出，回放结果会与旧版不同。
- 分类只看命令文本，偏差有两个方向。丢原文的方向：不认识的读取（例如 `python -c` 读文件、自写脚本打印文件）按运行类，跑通时会被摘成桩行，模型要原文得 `expand`。多留原文的方向：管道里的运行类、循环与条件、`git tag -l 'v*'` 这类带位置参数的列举都不改档。`[[ a > b ]]`、`(( x > 3 ))` 里引号外的比较符会被当成写文件，判成运行类，也落在丢原文的方向。
- `apps/sage` 的 e2e 用真实 bash 与 edit 工具驱动 `--scorer rule --scorer-apply entry`。bash 改名会让它变红；edit 等 echo 类缺省是 keep，改名只改变类别，不改变视图，这条测试看不出来。

## Testing

- `packages/plugins/test/scorer/bash-kind.test.ts`：常见读取与运行命令；git 的列举形式与写操作；`sed -i`、`sort -o`、`find -delete`；heredoc 写文件与输出重定向；引号内与转义的 `>`；`2>`、`>&2`、`/dev/null`；`cd` 前缀；空白与换行；读取管道的过滤；运行类接管道判混合、带 `pipefail` 判运行类；读取喂给运行；`&&` / `;` / `||` 链；中性的 `echo`；引号里的运算符；环境变量前缀；包装命令；循环与条件；注释。
- `packages/plugins/test/scorer/rule.test.ts`：fixture 里 echo 类用真实的一行结果。经 `contextCompiler` 的入口模式，跑通的运行类是 `summary` 和 `scorer:entry:summary;scorer:rule:bash-runner`，其余每一类（含接管道的运行类）的清单行与视图与不装打分器时相同，`entryScored.calls` 为 0；跑通的运行类在视图里正好是一行桩；下一轮冻结后桩行逐字节不变，`rebuildFromLog` 重建出同一行；`tiers` 覆盖；重建模式下 edit 保持 `kept`、运行类判 summary，不写 `manifest.scorer`。
- `packages/plugins/test/scorer/compiler.test.ts`：重建模式下假模型打分器给出没有笔记的 summary，视图只显示桩行。
- `apps/sage/test/e2e.test.ts`：真实 bash 与 edit 工具、`--scorer rule --scorer-apply entry`，bash 输出不少于 300 token；断言最后一份清单上 bash 块为 summary、reason 为 `scorer:entry:summary;scorer:rule:bash-runner`，edit 块为 full、reason 不含 `scorer:`。
- `apps/sage/test/scorer-apply.test.ts`：不写 `--scorer` 时退出 2 并点名 `--scorer rule|model|jev`；`--scorer rule --scorer-apply entry` 现在正常跑完，退出 0。这条用例原先钉的是被淘汰的拒绝行为。
- `pnpm check` 全绿。
