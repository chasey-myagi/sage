# View seam、最小 loop 与 hook 管线

## Problem

Log 只记发生过什么，不负责「这一轮模型看见什么」。没有 View seam 时，调用前没有一份唯一的 head + messages，也没有人在发出去之前检查 tool 是否成对、头有没有在阶段中途被改掉。如果 hook 还能改同一份数组，模型看见的内容和 Log 就会分成两份。

## Decision

每次模型调用前，内核向 `ViewProvider` 要一份 `CompiledView`。它只做两件事：`assertPaired`（不成对抛 `UnpairedToolCallError`）和 `assertHeadFrozen`（阶段内头的 system / tools hash 变了就抛 `HeadChangedMidEpochError`）。交给 `observe("view_compiled")` 的是 `structuredClone`。然后按 hash 追加 `request_header`，再 `stream`。`system` / `tools` 全文只在与 path 上前一条 `request_header` 的对应 hash 不同、或还没有前一条时写入。

工具管线固定为 `tool_pre`（可拒绝、可改参）→ `guard`（只能返回否决原因）→ `around` 包住 `execute` → `tool_post`（可替换结果）→ `tool_result`（只观察）。拒绝和未知工具都变成 `isError` 的 tool_result，不抛、不中断轮次。`observe` 抛错记入 `RunSummary.observerErrors`，run 继续。

默认投影是 `passthroughView`：按 path 上的 message 顺序投影；悬空 tool_use 补 `synthesizeToolResult(id, "TOOL_OUTCOME_UNKNOWN")` 并记 `dangling_tool_use`；孤儿 tool_result 丢弃并记 `orphan_tool_result`。`cacheBreakpoints` 原样透传，直通实现固定为 `["head"]`。head 用 session 注入的 `{system, tools}`。

`message` kind 的 payload 类型改为 `@sage/provider/contract` 的 `Message`（type-only）。校验仍只要求 `role` 是字符串且含 `content`。

## Alternatives considered

seam 上断言，不在 seam 上修复。崩溃留下的半截历史是投影的事：事实已经残缺，passthrough 补一条 `TOOL_OUTCOME_UNKNOWN` 并把修过写进 `manifest.anomalies`。插件把视图切坏是编译器的 bug，当场抛才能被看见。内核如果在发出去之前偷偷补上结果，一次坏的编译会变成一次成功的请求，清单上也没有「修过」的痕迹。两件事不互相替代。

guard 只能否决，不能放行。`tool_pre` 已经拒绝的调用，后面的 guard 即使不返回原因也不能让它执行。如果 guard 能把拒绝改成放行，结果就取决于谁后注册，V6 要消掉的正是这种顺序。guard 的返回只有原因或空，没有 allow。

`request_header` 按内容寻址，不每轮存全文。同一份头每轮都写进 Log，会把阶段内「头没变」这件事淹没在重复正文里。只在 hash 相对上一条变了、或还没有上一条时写全文；重复轮次只留 hash。重建时沿 path 找回最近一份全文，hash 对得上，就是当时发出去的头。

## Consequences

头只有在 `view.epoch` 大于上一条 `request_header` 当时的 epoch、并且这个 epoch 已经作为 `view_epoch` 写在 Log 上时才能变。插件把 `view.epoch` 改大但不追加 `view_epoch`，改头会被拒绝。

`observe` 的错误不进 Log。活的失败是运行时事件，`RunSummary.observerErrors` 是唯一出口。断言失败先追加 `terminal {outcome:"error"}` 再抛；适配器 `finish.kind === "error"` 返回 `outcome: "error"`，不抛。`maxTurns` 默认 50。`stop` 与 `max_tokens` 都结束为 `done`。

`collect` 在每轮里动态 import。contract 包的导出仍指向 TypeScript 源，静态 import 会让 `gen-log-manifest` 在纯 Node 下加载 `dist/index.js` 时解析失败。core 增加了对 `@sage/provider` 的 workspace 链接，没有第三方依赖。

本票不做压缩、第一方工具、录制回放和 steer。

## Testing

`pnpm check` 退出码为 0（lint、typecheck、分层、README、log manifest、全部测试）。新测试 25 个，覆盖不成对视图、阶段内改头、L4 重建、内容寻址、工具管线、observer 错误、视图拷贝、abort、`maxTurns`、悬空 tool_use 投影。红灯时这 25 个全部失败：`AgentSession is not a constructor`，以及 `UnpairedToolCallError` 尚未导出。
