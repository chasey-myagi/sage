# Agent Note: 无法解析的工具参数回放成 `{}`，工具结果引出原文

## Problem

模型流出的工具参数不是合法 JSON 时，`collect` 保留 `rawInput`、把 `input` 置为 null，这一步符合约定。但下一轮组请求时，两个 adapter 都把这条 assistant 消息原样回放：openai-chat 把 `rawInput` 写进 `function.arguments`，anthropic-messages 写 `input: null`。服务端以 400 拒收整条请求，400 属于不可重试类，之后每一轮都失败，会话以 error 结束。一次格式错误就让会话无法恢复。

同时，工具执行拿到的是 null，各工具只报 `input must be an object`。回放修好之后模型看到的是自己发了 `{}`，再配一句「必须是对象」，它看不出自己到底错在哪。

## Decision

- **回放规则下沉到 contract 一处定义**：`packages/provider/src/contract/tool-input.ts` 的 `replayToolInput(block)` 返回 `{ input, text }`。`block.input` 是 JSON 对象时，`input` 是它本身，`text` 是 `rawInput` 原文（空白时为 `{}`）；否则两者都是占位 `{}`（`EMPTY_TOOL_ARGUMENTS`）。openai-chat 取 `text`，anthropic-messages 取 `input`，两个 adapter 写法对称。占位只改发出去的副本，Log 里的 `rawInput` 不变。
- **L4**：占位是对 Log 里那条 assistant 消息的纯函数，发出去的请求仍能只凭 Log 重建，不需要新的 entry。两个 adapter 的文件头写明这条规则。
- **工具结果由 core 统一给出**：`HookDispatcher.runTool` 在 tool_pre 与 guard 之后，若生效的 input（含 `updatedInput`）不是 JSON 对象，就不执行工具，返回 `isError: true` 的 `<工具名>: arguments are not a JSON object, so the tool did not run. Resend the call with arguments that match its input schema. Received: <原文>`，原文最多引 `ARGUMENTS_PREVIEW_CHARS`（500）个字符。回放发的是 `{}`，模型只能从这里看到自己发了什么。
- **`collect` 把空白参数解析为 `{}`**：不带参数的调用（Anthropic 对无参工具流出空的 `partial_json`）以前得到 null。不改的话，上面的检查会把合法的无参调用判成错误。

## Alternatives considered

- **openai-chat 发 `JSON.stringify(input)` 而不是 `rawInput` 原文**：两个 adapter 更像，但会改变合法参数回放的字节（空白、键序），破坏前缀缓存。只在占位时换文本。
- **回放时丢掉这条 tool_use 和配对的 tool_result**：模型就看不到自己出过错，还要在 normalize 里同时改两条消息、维护配对，范围更大。不选。
- **在 `collect` 里直接把坏参数的 `input` 设为 `{}`**：Log 里就分不出「模型发了空对象」和「模型发了坏 JSON」，工具会带着空参数执行。不选。
- **各工具自己改错误信息**：每个工具都要知道 `rawInput`，同一句话写好几份；插件和外部工具也得跟着改。放在 dispatcher 一处。

## Consequences

- 模型可见文本变化：坏参数的调用在下一轮请求里显示为 `{}`，对应的 tool_result 是上面那条错误。合法参数的回放字节不变。
- 任何 input 不是对象的调用都不再进入 `execute`。工具 schema 按约定都是 object（两家 API 都要求），这条与之一致；tool_pre 仍可用 `updatedInput` 换成对象。
- `ToolUseBlock` 的约定多了一条：空白的 `rawInput` 对应 `input: {}`。

## Testing

- `packages/core/test/regressions/issue-172.test.ts`：经真实 adapter 与 `scriptedSseFetch`，第一轮流出截断的参数 `{"q":"half`。两个 adapter 各两例：第二轮请求体里的参数是 `{}`；Log 里保留原文，tool_result 为 `isError: true` 且引出原文，run 以 `done` 结束。修复前 4 例全红（openai-chat 回放原文，anthropic-messages 回放 null，tool_result 为空成功）。`scriptedSseFetch` 为此增加 `requestBodies()`。
- `packages/provider/test/collect.test.ts`：空白参数解析为 `{}`，修复前红。
- `packages/core/test/session/hooks.test.ts`：坏参数的调用不执行；tool_pre 用 `updatedInput` 换成对象后照常执行。
- `pnpm check` 全绿。
