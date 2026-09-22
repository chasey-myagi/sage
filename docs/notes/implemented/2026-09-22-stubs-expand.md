# 留桩与 expand 工具

## Problem

淘汰把 tool_result 换成 `[omitted]` 之后，模型只知道这里少了一段，不知道少的是什么，也不知道原文还在。重新调用原来的工具拿不回当时那一份：文件可能已经改过，bash 的输出也不能重现。没有按块 id 取回的路径，清单上也算不出 expand 率。

## Decision

档位 `none` 的工具块仍留下 tool_use，tool_result 的 content 换成一行桩：

```text
[#<chunkId> · <toolName> · <摘要> · <n>k tokens · omitted · expand("<chunkId>") to retrieve]
```

摘要是 tool_use 输入的紧凑单行，最长 80 个字符，超出截断并加上 `…`。`<n>k` 是该块原文 token 数除以 1000，保留一位小数。isError 不变。`[omitted]` 不再出现。

`expand` 是第一方只读工具，`accesses` 为空。`execute({ id })` 用和分块相同的 id 规则读 `store.path(sessionId)`：`seq:<seq>#<toolUseId>` 返回当时 tool_result 的 content，`seq:<seq>` 返回那条文本消息的原文。找不到则 `isError`，内容为 `Unknown chunk: <id>`。`apps/sage` 的 `createCodingSession` 把已创建的 store 和 sessionId 传进去。

取回的结果是新的工具块。下一轮它落在 recent 窗口里就是 `full`。被取回的原块仍是 `none`，桩还在；reason 在淘汰理由后追加 `;expanded_at:<turn>`，`<turn>` 是这条 expand 调用之前那条 manifest 的 turn，tier 不改。

清单 `budget` 增加 `omitted`（本轮 tier 为 none 的块数）和 `expands`（路径上 assistant 消息里 `name === "expand"` 的 tool_use 次数，累计到当前轮）。none 工具块在清单里的 `tokens` 改为桩文本的 token 数，原文估算记在 `originalTokens`。`budget.used` 仍只加非 none 的块，淘汰规则不动。`RunSummary` 不加这两个数。`rebuildFromLog` 用 `originalTokens` 和 Log 里的 tool_use 把同一行桩还原出来。

## Alternatives considered

桩上直接写 `expand("<id>")`，而不是让模型把原工具再调一遍。占位符的意思是「你可以重新执行」。文件和 bash 输出都对不住这个承诺，第二次拿到的字节和当时进 Log 的那一份不是同一件事。桩要说的是「原文还在，按这个编号来取」。把调用写进桩，模型不需要另记一种隐式协议。

expand 自己不写 Log。内核已经把每次工具调用记成 tool_use / tool_result。再追加一种 entry，同一次取回就有了两个来源。expand 率的分子就是这些普通调用的次数，分母是清单上的 omitted。工具只读路径。

## Consequences

本票只有 full 和 none 会真正出现在工具块上。summary / line 的自动生成仍是 #15。阶段判定和淘汰顺序都没改。

`budget` 对象多了两个字段。内核校验只要求 total、used、overrun 是有限数，多余字段能落盘，所以没有改 `packages/core`。`originalTokens` 同样只出现在编译器写的 chunk 记录上，内核把它当不透明 JSON。

桩写在历史中间，和 #9 的 `[omitted]` 一样会改前缀。前缀稳定性仍由 #12 负责。expand 率进 RunSummary 是 #17，这里只进清单。

## Testing

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、172 个测试）。红灯时 `stub.ts` / `expand.ts` 无法加载（`ERR_MODULE_NOT_FOUND`）；apply 测试收到的仍是 `[omitted]`；预算对象还没有 `omitted` / `expands`；e2e 第三轮的 expand 结果是 `Unknown tool: expand`。回放用例在预算 8000 下没有块被省略，请求与 passthrough 仍逐字节相同，没有重写期望文件。
