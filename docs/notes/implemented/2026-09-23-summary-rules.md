# 打分器摘要规范

## Problem

darkreader-6747 里，一次 1363 token 的 `read` 被压成 `Earlier file read (path under workspace src) already reviewed; contents not needed further.` 路径、符号和行号都没了，后面的轮次无从引用。同一批里能用的摘要是 `src/utils/url.ts: isURLMatched/createUrlRegex, isURLInList, isFullyQualifiedDomain`。指令只说 summary 可选、最长 200 字。

## Decision

`modelScorer` 的指令补上摘要规范：文件读取写路径、关键符号或导出、以及行号范围；grep 或 ls 写命中路径与行号；命令输出写命令和结论性数字或状态。不要只写判断（已看过、不再需要、和之前一样），那种情况改判 `none` 或 `full`。示例改成 `src/utils/url.ts: isURLMatched, isURLInList (L12-40)`。

入口打分在采纳前校验笔记。没给 `summary` 字段仍保持全文，reason 不变。写了笔记但不足 20 字（含空串），或不含路径、符号或数字（斜杠、扩展名、数字、驼峰或下划线）时，该块按 full 处理，reason 为 `scorer:entry:full;summary-rejected`。`manifest.entryScored` 与 `runHealth.entryScored` 增加 `summaryRejected`，这些块同时计入 `full`。清单没写这个字段时健康度加 0。

## Alternatives considered

为什么不在解析器里把无效摘要改成 `full`。入口 reason 和计数在编译器。解析器改档之后，编译器看不到原来的 summary，写不出 `summary-rejected`。

为什么不在重建路径做同一套校验。issue 规定的 reason 是 `scorer:entry:full;summary-rejected`，这次的真机样本也是入口生效。重建路径的 `scorer:model` 不动。

为什么没给字段的 summary 不记 `summaryRejected`。#73 已经把「没给笔记」定义成保持全文、reason 仍是淘汰结果。空串和过短笔记才是写了但没用的摘要。

## Consequences

规则打分、影子打分、重建打分的档位不变。`summaryRejected` 加在 `full` 后面，不重排已有字段。日志校验在字段出现时要求它是有限数字，不要求每份清单都带它。

没加依赖。测试不打真网。

## Testing

先有失败测试：判断句 `already reviewed; not needed` 仍被收成 `scorer:entry:summary`；不足 20 字的 `src/a.ts:1` 同样被收成 summary；指令快照缺少路径、行号和「不要只写判断」。

`pnpm check` 通过：biome、tsc、分层、README、log manifest，vitest 85 个文件、412 个测试全绿。覆盖判断句与空串退回 full 且 `summaryRejected` 为 1、`src/utils/url.ts: isURLMatched, isURLInList (L12-40)` 被采纳、指令含那三句规范。两条回放记录的终端健康度补上 `summaryRejected: 0`。
