# grep 单行上限

## Problem

真机轨迹 `budget6000-deepseek-flash-20260923-0244.jsonl`（deepseek-flash，预算 6000）里，turn 4 的 `grep` 不带 path，模式是 `formatHits|optionalBool|searchWithoutRg`。16 条命中共 73,917 字符，其中 9 条来自 `packages/plugins/test/replay/fixtures/issue-46/long-deepseek-flash.jsonl`，最长四行 23,712 / 15,500 / 10,029 / 6,324 字符。turn 5 全价输入 24,549 token，`budget.used` 26,102，超支 20,102。编译器按冻结规则留到 turn 8 才淘汰，行为正确，但这份结果没有告诉模型那三个标识符在哪些源文件里。`grep` 已有 200 条和单文件 1MB 上限，没有单行长度上限。

## Decision

`GREP_MAX_LINE_CHARS = 300`。行不超过 300 字符时原样返回。更长时，匹配起点落在前 300 字符内就留开头 300；否则以匹配起点为界，前后各留 150。被截掉的一侧加 `…`，行末加 `[+N chars]`，N 是原文里没留下的字符数。`rg` 和进程内搜索都走 `clipHitText`，`rg` 的命中在格式化前按文件名和行号排序，和进程内路径同一份输出。不加参数。README 的 Model Experience 和 `pattern` 的 inputSchema 描述写明这个上限。

## Alternatives considered

复用 `truncate.ts` 不行。它的契约是不返回半行，并且按字节而不是字符截，长行会整行丢掉，匹配点也留不住。

把截断交给 `rg --max-columns` 不行。进程内回退没有这条旗标，两边的标记也不会相同；只留行首还会把落在 300 之后的匹配切掉。

加一个 `max_line_chars` 参数不行。这一票明确不加参数，模型也不该为了避开 JSONL 去调上限。

整行丢弃不行。模型需要看见匹配文本，才知道该去读哪个文件。

## Consequences

短于等于 300 字符的命中与原来逐字节相同。200 条和 1MB 文件上限不变。模型从长行里只能看到匹配附近的窗口，要全文得再 `read`。匹配文本长过 150 字符、且起点在 300 之后时，窗口在起点后 150 处截断。窗口位置用 JS `RegExp` 计算；`rg` 能跑但 JS 编不过的模式退化为留开头 300。

## Testing

`packages/tools/test/regressions/issue-51.test.ts` 覆盖三条验收：5000 字符中段命中的输出不超过 320 字符且带两端 `…` 与 `[+N chars]`，`rg` 与进程内输出相同；短于 300 字符的源码命中逐字节不变；在仓库根重放那次调用，结果短于 8000 字符。`pnpm check` 通过。
