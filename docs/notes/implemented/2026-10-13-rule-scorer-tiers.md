# Agent Note: 规则打分器按工具类型分档，bash 命令分读取类与运行类

## Problem

`ruleScorer` 是个空打分器：每个候选都判 full，带 `isNoop: true`，编译器见到它就跳过改写，入口模式下甚至不调用。`--scorer rule` 因此和不写 `--scorer` 没有区别，CLI 还专门拒绝 `--scorer rule --scorer-apply entry`。可是单凭工具类型就有很强的分档依据：edit / write / update_ledger 的回显只复述调用参数，之后几乎不再被用到；read 和 grep 的结果常被引用；bash 要看命令，读取类（`cat`、`sed -n`、`grep`、`ls`）和 read 相近，运行类（测试、构建、脚本）跑通后多半用不到，报错时例外。这些判断不需要发任何请求。

## Decision

- **`classifyBashCommand(command)` 放在 `packages/plugins/src/scorer/bash-kind.ts`，从包入口导出**，返回 `"reader" | "runner" | "mixed"`。纯函数，只看文本。规范化：丢掉 heredoc 正文，在引号外按 `&&`、`||`、`;`、换行切成部分、按 `|` 切成管道级，压缩空白，去掉开头的括号和 `NAME=value`。`cd`、`echo` 这类中性命令整级丢掉，这就是「去掉 `cd … &&` 前缀」的一般形式。管道按第一级判，后面的读取类级只是过滤（`pnpm test | tail` 是运行类）。读取类、中性命令、git 读取子命令、`find` 的动作旗标、`sed -i` 都是带 JSDoc 的具名常量；不认识的命令一律按运行类，写文件的输出重定向也是运行类。
- **`ruleScorer` 改成工厂 `ruleScorer({ tiers? })`**。候选先归七类之一（`echo`、`reader`、`bash-reader`、`bash-runner`、`bash-runner-error`、`bash-mixed`、`other`），再查 `DEFAULT_RULE_TIERS`：echo 判 none，bash-runner 判 summary，其余是 `keep`。`keep` 不给判定，块留在编译器原有规则的档位和 reason 上。给了判定的 reason 是 `scorer:rule:<类别>`；入口模式下照 `entryReason` 的现有组合写成 `scorer:entry:<tier>;scorer:rule:<类别>`。
- **summary 不生成文本**：`ScoreDecision` 新增 `stubOnly?: true`。入口打分见到它就写成 summary 行、笔记为空串、不做可定位检查；重建打分同样写空笔记。`renderSummary` 遇到空笔记只输出桩行，所以视图、冻结后的下一轮和 `rebuildFromLog` 重建出来的都是同一行桩。
- **`isNoop` 删掉，换成 `isLocal?: true`**：含义是「只看候选、从不发请求」。它现在只有一个用途：离线回放保留这样的打分器，其余的换成缺省档位的 `ruleScorer()`，报告照旧写 `scorer:offline-rule`。
- **`contextCompiler` 的 `scorer` 默认不装**。原来的默认是空的 `ruleScorer`，「没装打分器时与只跑淘汰逐字节相同」靠的就是它；现在规则打分器真会改档，默认值改成 undefined，重建和入口两条路径在没有打分器时直接跳过。CLI 不写 `--scorer` 的行为不变。
- **入口打分成功时的 `calls` 走 `requestCount`**：规则打分器返回 `requested: false`，`entryScored.calls`、`shadow.calls` 都记 0，重建路径没有 usage 也没有 fallback，不写 `manifest.scorer`。`ScoreResult.requested` 的 JSDoc 从「发请求前就放弃」放宽为「没发请求」。
- **CLI**：`--scorer-apply entry` 现在只要求写了 `--scorer`，任何一个都行；报错改为点名 `--scorer rule|model|jev`。

## Alternatives considered

- **summary 的笔记写成命令本身或输出末几行**：issue 要求不额外生成文本；命令已经在桩行里，末几行是另一种摘要策略，属于 jev 那条线。
- **空串笔记在入口打分里一律当成只显示桩行**：不需要新字段，但会改掉模型打分器「空串笔记被拒、保持 full」的既有行为（`issue-87` 回归钉着它）。用显式的 `stubOnly` 只放行明确声明的判定。
- **runner 直接判 none**：效果几乎相同，但 issue 要的是 summary 档，后续标签工作要区分「回显丢弃」和「运行输出摘要」两种判定。
- **保留 `isNoop`、让编译器默认仍装规则打分器**：默认编译会悄悄开始改档，所有不写 `--scorer` 的会话与历史清单都会变；默认改成不装打分器，变化只落在显式选了 `rule` 的地方。
- **离线回放把调用型打分器换成「不装」**：标签 `scorer:offline-rule` 就不再属实，要另起一个标签；换成真正的规则打分器，标签字面上仍对，回放结果也更接近一个会改档的策略。
- **工具名从 `@sage/tools` 的 `TOOL_NAMES` 导入**：分层规则不许 plugins import tools，所以在 `rule.ts` 里写成字面量并注明对应关系。

## Consequences

- `--scorer rule` 会改变模型看到的消息：回显变成桩，跑通的运行类 bash 只剩桩行。入口模式下这些块从第一次进入视图起就是最终形态；重建模式下只在阶段重开时生效。
- 任何打分器给出的空笔记 summary 现在都只显示桩行，不再留下一个后面没有内容的 `summary (model notes, not evidence): ` 标签。这改的是重建模式下模型打分器漏写笔记时的视图。
- 离线回放 jev / model 策略时，档位由规则打分器给出，回放结果会与旧版不同。
- 分类只看命令文本：引号外的 `>=` 会被当成重定向，`python -c` 读文件会被当成运行类。偏差都朝保守方向（运行类报错或混合时保留原文），代价是少省一些 token，不会丢证据。分类器对外导出，后续的标签工作在 apps 里复用同一套规则。

## Testing

- `packages/plugins/test/scorer/bash-kind.test.ts`：常见读取命令与运行命令、`cd` 前缀、空白与换行、管道过滤、管道里读取喂给运行、`&&` / `;` / `||` 链、中性的 `echo`、引号里的运算符、环境变量前缀、heredoc 写文件与输出重定向。
- `packages/plugins/test/scorer/rule.test.ts`：经 `contextCompiler` 的入口模式，断言每一类在清单上的 tier 与 reason，`keep` 类的清单行与不装打分器时逐字段相同、原文未动，`entryScored.calls` 为 0；跑通的运行类在视图里正好是一行桩；下一轮冻结后桩行逐字节不变，`rebuildFromLog` 重建出同一行；`tiers` 覆盖；重建模式下同样的档位与 `scorer:rule:<类别>` reason，且不写 `manifest.scorer`。
- `apps/sage/test/scorer-apply.test.ts`：不写 `--scorer` 时退出 2 并点名 `--scorer rule|model|jev`；`--scorer rule --scorer-apply entry` 现在正常跑完，退出 0。这条用例原先钉的是被淘汰的拒绝行为。
- `pnpm check` 全绿。
