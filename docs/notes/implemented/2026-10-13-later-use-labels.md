# Agent Note: 离线标签分成 touch 与 content，stats 与 calibrate 同时输出

## Problem

`sage stats` 的影子对账和 `sage calibrate-jev` 的扫描共用一个「后来需要」标签：块之后被 `expand`，或者它是 `read` 的结果、同一路径在之后的轮次又被 `read` 或 `edit`。这个标签量的是「路径又被碰到」，不是「内容被用到」：

- 模型改文件前照例重读，read 块因此大量标成需要，假阳性多；
- grep 与 bash 块不在这条规则里，只能靠 `expand` 标上。passthrough 会话从不省略，模型没理由 `expand`，grep 的标签在那里恒为 false，按这一层算的指标没有意义。

## Decision

- **两个标签一起算，代码只有一份。**新文件 `apps/sage/src/later-use.ts` 导出 `laterUseIn(entries)`，返回 `(chunkId, at, turn) => { touch, content }`。原来写在 `stats.ts` 里的 `neededLaterIn` 及其辅助函数挪进来，口径不变，改名为 touch。`stats.ts` 与 `calibrate-jev.ts` 都调它。
- **content 的定义。**满足任一条：之后被 `expand`；之后某次调用用到了首次由这块输出引入的字符串；块是 bash，之后又跑了规范化后相同的命令。用到的字符串是：`edit` 的 `old_string`，grep 工具的 `pattern`，bash 里 grep / rg 各级的模式，`read` / `edit` / `write` 的 `path`。「首次引入」按全文首次出现的位置判：路径上所有消息的文字（用户与 assistant 文字、thinking、工具输入的每个字符串值、工具结果）按 Log 顺序拼成一份，查询字符串第一次出现的位置落在某块的工具结果里、且在发出调用的那条 assistant 消息之前，就记给那一块。任务或更早的输入输出里已有的字符串不记给任何块。少于 3 个字符的字符串不计。
- **计算方式。**一次扫描拼出全文与各段起点；每个不同的查询字符串做一次 `indexOf` 并缓存，二分查找落在哪一段；每块只记最后一次记给它的调用位置，查询时与判定点比较。复杂度 O(N + Q·N)，N 是全文字符数，Q 是不同查询字符串数。未经变换的查询字符串就在发出它的调用的输入里，扫描最晚停在那里。重跑按规范化命令分组。
- **bash 规范化复用分类器的扫描。**新文件 `packages/plugins/src/scorer/bash-search.ts` 导出 `normalizeBashCommand` 与 `bashSearchPatterns`，都调用 `bash-kind.ts` 里 `classifyBashCommand` 已有的引号感知切分（`splitParts`、`commandWords`，改为模块内导出），没有第二份解析；`bash-kind.ts` 因此不超过 600 行。比较键保留 heredoc 正文，分类时则丢掉，否则两次写入不同内容的命令会被当成重跑。
- **输出。**`sage stats` 的 `laterNeeded` / `precision` 换成 `touch` 与 `content` 两个 `{ laterNeeded, precision }`。`sweep.json` 的块行把 `needed` 换成 `touch` 与 `content` 两列，并按评审要求加上 `turn` 与 `ledgerEmpty`。扫描行和推荐按 `--label touch|content` 选的标签算，默认 content；`sweep.json` 与汇总行都写出所用的 `label`。

## Alternatives considered

- **逐块检查「之后的输入里是否出现了它输出里的字符串」**：每块都要和之后所有调用比一遍，几百轮的会话是 O(块数 × 调用数 × 文本长度)。按查询字符串找首次出现，每个字符串只扫一次，还顺带满足「首次引入」：重读的块拿不走功劳。
- **用后缀自动机或 Aho-Corasick 建索引**：最坏复杂度更好，但要手写或引入依赖。查询字符串在自己的调用里必然出现，原生 `indexOf` 的扫描有上界，会话规模下够用。
- **给 sweep 行加 `policy`**：Log 里没有记录会话策略，只能从清单形状（passthrough 清单没有 `budget`）推断，等于用缺字段来编码状态。`ledgerEmpty` 直接取自重建出的 Jev 请求，正是评审关心的输入分布差别；代价是 compiler 会话在第一次 `update_ledger` 之前的块也是 true，配合 `turn` 可以区分。
- **扫描同时输出两套行**：`sweep.json` 体积翻倍，推荐也要两份。块行已有两列，换标签重跑命中缓存、不花钱。
- **bash 重跑只记给最后一次之前的那一次**：「之后又跑了同一命令」本身就是定义，几次重跑之间无法从 Log 区分哪一次的输出被用到。

## Consequences

- 两个标签都偏向大块和剩余轮数多的块：文本越多越容易含有之后用到的字符串，后面的调用越多越容易碰到同一路径。
- content 只认四种字符串和重跑。模型读后凭理解写的新代码、换了写法的模式、只写文件名或绝对路径的打开都认不出；短而常见的字符串首次出现在某块里就记给它。同一段文字读两次，功劳只给第一次。
- 调用的 `path` 只去掉开头的 `./`，不做目录拼接：`ls src` 列出的 `a.ts` 之后以 `src/a.ts` 打开时不算。
- 系统提示不在 Log 的消息里，不进全文；系统提示里写过的路径会被记给第一次输出它的块。
- `sage stats` 的输出字段改名，读旧字段的脚本要跟着改。预发布阶段不保留旧字段。

## Testing

- `apps/sage/test/later-use.test.ts` 用 faux provider 真跑 compiler 会话，再给单个块补一条影子 `none`，跑 `sage stats` 读两个标签：
  - 读 `a.txt`、用 bash 追加内容、重读、`edit` 的 `old_string` 只在新内容里：旧块 touch 为 1、content 为 0；重读块 content 为 1，而 touch 为 0，因为编辑紧跟在它的判定轮里；
  - grep 工具的模式和 bash `rg -e` 的模式分别来自前两块的输出，两块都记 content；模式已写在任务里时，含它的块不记；
  - 同一条 bash 命令多空格、带注释重跑，前一次记 content，最后一次不记；
  - grep 输出里点名的路径之后被 `read`，grep 块记 content。
- `apps/sage/test/calibrate-jev.test.ts`：compiler 会话里每块的 touch / content 两列；passthrough 会话里 grep3 的 content 为 true（之后的 `edit` 用了它输出里的行），所有块 `ledgerEmpty` 为 true；默认按 content 扫描、`--label touch` 切换后 `needed` 不同，`sweep.json` 与汇总都写出 `label`；`--label` 写错打印 usage 并退出 1。
- `apps/sage/test/shadow-stats.test.ts` 在真实 bench 轨迹上 touch 的计数与改名前相同。
- `packages/plugins/test/scorer/bash-kind.test.ts` 覆盖 `normalizeBashCommand` 的相等与不等、`bashSearchPatterns` 的 `-e`、带值选项、`--`、包装命令与多级管道。
- 测试不打真网，不调真实 Jev，Jev 只经 `jsonFetch` 替身。`pnpm check` 全绿。
