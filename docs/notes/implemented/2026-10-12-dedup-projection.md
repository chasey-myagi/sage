# Agent Note

## Problem

plugins 里有三处复制了别处已经拥有的事实：

- `compiler/log-read.ts` 的 `projectionSeqs` 把 core `passthrough.ts` 的投影算法（开放 tool_use 跟踪、孤儿 tool_result 丢弃、悬空补齐）重写了一遍，只为得到「投影后第 i 条消息来自哪个 `seq`」。`compiler.ts` 与 `subtask-view.ts` 先跑 `passthroughView.view()`，再跑这份副本，用 `assertSeqsMatch` 比长度。core 的修复逻辑一改，副本就会悄悄错位。
- 从块 id 取 toolUseId 有三份：`compiler/rebuild.ts`、`replay/offline.ts`、`apps/sage/src/stats.ts`，都按第一个 `#` 切。边界不一致：stats 要求 `#` 前非空，另两份接受 `#x`。`expand.ts` 又用自己的正则解析同一个 `seq:<seq>#<toolUseId>` 格式。
- `stats.ts` 写死 `"expand"`，`spawn.ts` 里 `spawn_subtask` / `spawn_subtasks` 各在工具名与错误文本里重复出现。

## Decision

- passthrough 把「`manifest.chunks[i]` 对应 `messages[i]`；投影消息的 chunk id 是源 entry 的 id；补出来的 tool_result 消息的 chunk id（`dangling-before:<下一条 entry id>` / `dangling:<tool_use id>`）不对应任何 entry」写进文件头与 `passthroughView` 的 JSDoc，`test/view/passthrough-chunk-ids.test.ts` 钉住。
- `projectionSeqs` 与 `assertSeqsMatch` 删掉，换成 `projectedSeqs(path, chunks)`：建 `entry.id → seq` 映射，查不到的记 -1。长度由 passthrough 的契约保证，不再单独断言。
- `chunks.ts`（块 id 格式的拥有者）加 `parseToolChunkId` 与 `toolUseIdOfChunk`。后者先去掉 `own:` 前缀，再严格匹配 `seq:<seq>#<toolUseId>`。三处调用方与 `expand.ts` 都改用它；`toolUseIdOfChunk` 与 `EXPAND_TOOL_NAME` 从 plugins 入口转出给 stats。
- `spawn.ts` 的两个工具名提成 `SPAWN_SUBTASK_TOOL_NAME` / `SPAWN_SUBTASKS_TOOL_NAME`，工具名与错误文本逐字节不变。

疑似 bug（`spawn.ts` 的 `child:<sessionId>#<chunkId>` 引用被按第一个 `#` 切错）不成立：这三处解析器只读 manifest 的 `chunks[].id` 或 `shadow.decisions[].id`，这些 id 都来自 `indexChunks`（`seq:…`）或子任务视图的 `own:seq:…`。`child:` 引用只出现在 `spawn_subtask` 的 tool_result 文本和模型发出的 `expand` 输入里，`expand.ts` 用自己的 `child:` 正则先拆开。新解析器对 `child:` 引用返回 undefined，而不是切出 `seq:3#toolu…`，用例里钉住了。

## Alternatives considered

- 把 passthrough 的 `project()` 导出给 plugins 用。会把修复算法的内部形状变成跨包 API；manifest 的 chunk id 本来就在 Log 里，是已有的契约面。
- 按 chunk id 前缀（`dangling`）判断补出消息。那是把状态编码进字符串；查 entry id 映射不依赖前缀写法。
- 保留按第一个 `#` 切的宽松解析。会接受 `#x`、`child:…` 之类不是块 id 的东西，三处边界也继续不一致。

## Consequences

- passthrough 改 chunk id 的写法时，必须保持「投影消息用 entry id、补出消息不撞 entry id」，否则 `projectedSeqs` 会错。契约与测试都在 core。
- 新的块 id 格式要在 `chunks.ts` 同时改格式与 `parseToolChunkId`。
- 不是 `seq:<n>#<id>`（可带 `own:`）形状的 id，`rebuild` 不再为它生成 stub，`offline` 与 `stats` 不再把它当工具块。生产里不存在这种 chunk id。

## Testing

- `pnpm check` 全绿（95 个测试文件、503 个用例）。快照、回放、`issue-*` 回归用例未改动，原样通过，模型可见内容不变。
- 新用例：`names each passthrough chunk after its source entry and gives synthesized messages ids that name no entry`、`reads the tool_use id of local and own tool chunk ids and rejects prose ids and child references`。
- 对打包产物调用 `spawn_subtask` / `spawn_subtasks` 的各条错误路径，工具名与错误文本和改动前的字面量逐字一致。
