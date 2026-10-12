# Agent Note: 离线标签分成 touch 与 content，stats 与 calibrate 同时输出

## Problem

`sage stats` 的影子对账和 `sage calibrate-jev` 的扫描共用一个「后来需要」标签：块之后被 `expand`，或者它是 `read` 的结果、同一路径在之后的轮次又被 `read` 或 `edit`。这个标签量的是「路径又被碰到」，不是「内容被用到」：

- 模型改文件前照例重读，read 块因此大量标成需要，假阳性多；
- grep 与 bash 块不在这条规则里，只能靠 `expand` 标上。passthrough 会话从不省略，模型没理由 `expand`，grep 的标签在那里恒为 false，按这一层算的指标没有意义。

## Decision

- **两个标签一起算，代码只有一份。**新文件 `apps/sage/src/later-use.ts` 导出 `laterUseIn(entries)`，返回 `(chunkId, at, turn) => { touch, content }`。原来写在 `stats.ts` 里的 `neededLaterIn` 及其辅助函数挪进来，口径不变，改名为 touch。`stats.ts` 与 `calibrate-jev.ts` 都调它。
- **content 的定义。**满足任一条：之后被 `expand`；之后某次调用用到的字符串由这块输出提供；块是 bash，之后又跑了规范化后相同的命令。全文是路径上所有消息的文字（用户与 assistant 文字、thinking、工具输入的每个字符串值、工具结果）按 Log 顺序拼成的一份，只看发出调用的那条 assistant 消息之前的部分。出处落在某块的工具结果里就记给那一块，落在用户或 assistant 文字、工具输入里就谁都不记。少于 3 个字符的字符串不计。
- **抄写的原文按最近出处，名字按首次出处。**
  - `edit` 的 `old_string` 是照着最近一次看到的文件逐字抄的，取调用之前最近的副本：文件重读过，功劳给重读的块；同一行先出现在 grep 命中里、之后又被 read 读到，功劳给 read。
  - 路径与搜索模式是名字：grep 工具的 `pattern`，bash 里 grep / rg 各级的模式，`read` / `edit` / `write` 的 `path`，bash 读取类各级打开的文件。名字取全文首次出现的位置。理由是名字一旦出现就是已知的，之后的列表、重读或 grep 再列出它，并没有提供新信息。实测这一点：名字也按最近出处时，passthrough 会话里先 read、再 grep 同一文件、再重读的序列，会把重读记给中间那次 grep，正是 touch 被诟病的「再碰即算」。按首次出处时，任务里已经写过的名字自然不记给任何块，不用另加规则。
- **路径按三种写法查。**真实会话里 read / edit / write 多用绝对路径，工具输出里的路径却相对于工作区。所以依次试：相对工作区根目录的写法（根目录取 `request_header.system` 最后一行 `Workspace: <abs>`，前缀常量 `WORKSPACE_LINE_PREFIX` 由 `coding-session.ts` 的系统提示导出，两边共用）；原样（去掉开头的 `./`）；至少两段的后缀，从长到短。哪种写法在调用之前有副本就用哪种，后面的不再试。
- **bash 读取也算打开。**`packages/plugins/src/scorer/bash-search.ts` 新增 `bashFileOperands`，取 `cat`、`head`、`sed -n` 等读文件内容命令的文件操作数。它用 `bash-kind.ts` 的 `commandWords` 与新导出的 `isReaderStage` 判断该级是不是读取类，没有第二份解析。
- **计算方式。**一次扫描拼出全文与各段起点。名字每种写法做一次 `indexOf` 并缓存；`old_string` 从调用位置向前做一次 `lastIndexOf`，停在最近的副本。再二分查找副本落在哪一段。每块只记最后一次记给它的调用位置，查询时与判定点比较。复杂度 O(N + F·N)，N 是全文字符数，F 是要查的写法数。
- **bash 规范化复用分类器的扫描。**`bash-search.ts` 还导出 `normalizeBashCommand` 与 `bashSearchPatterns`，都调用 `bash-kind.ts` 已有的引号感知切分（`splitParts`、`commandWords`，改为模块内导出）；`bash-kind.ts` 因此不超过 600 行。比较键保留 heredoc 正文，分类时则丢掉，否则两次写入不同内容的命令会被当成重跑。grep 与 rg 的带值选项分两张表：rg 的 `-r`、`-E` 带值，grep 的同名选项是开关。
- **输出。**`sage stats` 的 `laterNeeded` / `precision` 换成 `touch` 与 `content` 两个 `{ laterNeeded, precision }`。`sweep.json` 的块行把 `needed` 换成 `touch` 与 `content` 两列，并按评审要求加上 `turn` 与 `ledgerEmpty`。扫描行和推荐按 `--label touch|content` 选的标签算，默认 content；`sweep.json` 与汇总行都写出所用的 `label`。

## Alternatives considered

- **四种字符串都按首次出处**：重读过的文件，`old_string` 的功劳会落在第一次 read 上；`old_string` 先出现在 grep 命中行里时会落在 grep 上，最近那次 read 拿不到。评审在真实会话上看到 read 的 content 因此系统性偏低。
- **四种字符串都按最近出处**：名字会被之后任何再列出它的块抢走，见 Decision 里的反例；任务里写过的名字也要另加一条排除规则。
- **逐块检查「之后的输入里是否出现了它输出里的字符串」**：每块都要和之后所有调用比一遍，几百轮的会话是 O(块数 × 调用数 × 文本长度)。按查询字符串定位出处，每次只扫一遍。
- **用后缀自动机或 Aho-Corasick 建索引**：最坏复杂度更好，但要手写或引入依赖。名字一定出现在自己调用的输入里，`indexOf` 的扫描有上界；`lastIndexOf` 通常在最近读过的块就停。
- **路径只按字面匹配**：绝对路径与工具输出里的相对路径永远对不上。
- **给 sweep 行加 `policy`**：Log 里没有记录会话策略，只能从清单形状（passthrough 清单没有 `budget`）推断，等于用缺字段来编码状态。`ledgerEmpty` 直接取自重建出的 Jev 请求，正是评审关心的输入分布差别；代价是 compiler 会话在第一次 `update_ledger` 之前的块也是 true，配合 `turn` 可以区分。
- **扫描同时输出两套行**：`sweep.json` 体积翻倍，推荐也要两份。块行已有两列，换标签重跑命中缓存、不花钱。
- **bash 重跑只记给最后一次之前的那一次**：「之后又跑了同一命令」本身就是定义，几次重跑之间无法从 Log 区分哪一次的输出被用到。

## Consequences

- 两个标签都偏向大块和剩余轮数多的块：文本越多越容易含有之后用到的字符串，后面的调用越多越容易碰到同一路径。
- bash 里只取读取类各级的文件操作数作为「打开」，有意比「命令里出现的路径词都算」更窄：后者会把 `node test.js`、`git diff a.ts` 也记作读了内容。
- content 只认上述字符串和重跑。模型读后凭理解写的新代码、换了写法的模式都认不出；不在工作区下、又凑不出两段后缀的路径也认不出。短而常见的名字首次出现在某块里就记给它；两段后缀可能撞上别的目录下的同名文件。
- 工作区根目录依赖系统提示最后一行的格式。改系统提示时要保留 `WORKSPACE_LINE_PREFIX` 那一行，否则绝对路径只能靠后缀匹配。
- bash 读取类的文件按写法匹配，不跟踪 `cd` 到子目录。
- 系统提示不在 Log 的消息里，不进全文；系统提示里写过的路径会被记给第一次输出它的块。
- `sage stats` 的输出字段改名，读旧字段的脚本要跟着改。预发布阶段不保留旧字段。

## Testing

- `apps/sage/test/later-use.test.ts` 用 faux provider 真跑 compiler 会话，再给单个块补一条影子 `none`，跑 `sage stats` 读两个标签：
  - 读 `a.txt`、用 bash 追加内容、重读、`edit` 的 `old_string` 只在新内容里：旧块 touch 为 1、content 为 0；重读块 content 为 1，touch 为 0，因为编辑紧跟在它的判定轮里；
  - 同一文件不改动地读两次再 `edit`：功劳给重读的块，第一次 read 的 content 为 0；
  - grep 工具的模式和 bash `rg -e` 的模式分别来自前两块的输出，两块都记 content；模式已写在任务里时，含它的块不记；
  - 同一条 bash 命令多空格、带注释重跑，前一次记 content，最后一次不记；
  - grep 输出里点名的路径之后被 `read` 打开，相对路径、工作区下的绝对路径、`cd <root> && cat <相对路径>` 三种写法都让 grep 块记 content。
- `apps/sage/test/calibrate-jev.test.ts`：compiler 会话里每块的 touch / content 两列；passthrough 会话里 grep3 的 content 为 true（之后的 `edit` 用了它输出里的行），所有块 `ledgerEmpty` 为 true；默认按 content 扫描、`--label touch` 切换后 `needed` 不同，`sweep.json` 与汇总都写出 `label`；`--label` 写错打印 usage 并退出 1。
- `apps/sage/test/shadow-stats.test.ts` 在真实 bench 轨迹上 touch 的计数与改名前相同；content 多记了一块，即紧跟着被 `edit` 的 spec 文件 read。
- `packages/plugins/test/scorer/bash-kind.test.ts` 覆盖 `normalizeBashCommand` 的相等与不等；`bashSearchPatterns` 的 `-e`、带值选项（含 rg 的 `-r`、`-E`、`--color`、`--sort`，以及 grep 的 `-r`、`-E` 作开关）、`--`、包装命令与多级管道；`bashFileOperands` 的 `cat`、`sed -n`、`head -n`、`awk -F`、`sed -i`、通配符与重定向。
- 测试不打真网，不调真实 Jev，Jev 只经 `jsonFetch` 替身。`pnpm check` 全绿。
