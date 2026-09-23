# 工具错误不终止会话

## Problem

2026-09-23，入口生效 A/B 跑 `vuejs__core-11625`。模型用 `read` 去读工作区外的绝对路径，terminal 变成 `{"outcome":"error","error":"Path is outside the workspace: …"}`，50 轮会话作废。路径守卫抛了 `PathOutsideWorkspaceError`。编译器在下一轮视图里调用 `accesses()`，这个异常冒到 `AgentSession.run`，被当成不可恢复错误。工具自己 `execute` 里抛出的异常虽然已经收成 `isError` 的 tool_result，正文却只有 message，没有工具名；`runHealth` 也不计这些错误。

## Decision

内核执行工具时，`execute` 的同步抛错和被拒绝的 Promise，以及包住它的 `around` 抛错，都写成 `tool_result { isError: true, content: "<工具名>: <message>" }`，会话继续。只有内核自身的失败和 provider 的 finish error 才追加 `outcome: "error"` 的 terminal。

`resolveInside` 不再抛。越界返回 `fail("[error] Path is outside the workspace: <input>")`。`read` / `write` / `edit` / `grep` / `find` / `ls` 把这个结果原样交回，不读不写。`pathAccess` 对越界路径返回空列表，`accesses()` 因此也不抛。`glob.ts` 只编译匹配模式，没有工作区路径判断，不动。`bash` 的沙箱不在这次范围里。

连续多次工具错误仍不终止。`runHealth.toolErrors` 统计路径上 `isError: true` 的 tool_result 条数，5 次就是 5，不设杀死阈值。

## Alternatives considered

不在内核里按错误类名识别 `PathOutsideWorkspaceError`。core 不能认识 tools 的类型；编译器调用 `accesses()` 时，抛错的是工具自己。把越界收成 `fail` 之后，插件不用改。

不在第 5 次工具错误时终止。issue 要求连续 N 次（验收取 5）仍然继续，只把次数记进 health。加阈值会把「模型用错路径」再次变成整段会话作废。

不把工具自己返回的 `fail` 再包一层 `<工具名>:`。越界正文已经是 `[error] Path is outside the workspace: …`。前缀只加在真正抛出来的异常上，避免模型看见两层包装。

## Consequences

工具的 `execute` 和 `accesses` 都不许把路径越界抛出进程。新工具沿用 `resolveInside` 的 `{ ok: false, result }`。`accesses()` 在越界时是空列表：只读工具没有声明访问，写工具在并行子任务里会被当成未知写。这比杀掉会话窄。

`terminal.health` 多了 `toolErrors`。回放夹具里的健康对象要带上这个数字。历史 jsonl 少这个字段仍然能读，`runHealth` 会按 tool_result 重算。

`bash` 的网络沙箱仍可能在创建工具时抛 `ToolNetworkError`。那是启动失败，不是一次工具调用。

## Testing

`pnpm check` 是合并前的闸门。红灯时：假工具抛错的 tool_result 正文是 `nope` 而不是 `boom: nope`；五次错误后 `health.toolErrors` 为 `undefined`；`read` / `ls` / `grep` / `find` / `edit` / `write` 的 `accesses()` 抛 `PathOutsideWorkspaceError`。修完后这些断言变绿，越界正文是 `[error] Path is outside the workspace: …`，会话 terminal 不是 `error`。测试不打真网，不跑真机 bench。
