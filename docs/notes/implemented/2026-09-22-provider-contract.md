# Provider：本仓词表与两个适配器

## Problem

loop 不能站在别人的 provider 类型上。Anthropic Messages 和 OpenAI 兼容 chat 的线上形状不一样：一边是 content block 和 `cache_control`，一边是 `role:system` 和 `tool_calls`。词表如果偏向任何一边，另一边就会漏字段，上层还是得认识那家 SDK。缓存断点如果由适配器自己决定，前缀冻结就不再由请求说了算。

## Decision

`packages/provider/src/contract` 是本仓的词表：`Message` / `ToolDef` / `Usage` 四元组 / `StreamChunk` / `prepare` / `stream`。`collect` 是同目录纯函数，按 index 组装 block；`tool_input_delta` 拼成 `rawInput`，结束时 `JSON.parse`，失败则保留原文、`input` 为 `null`。

两个适配器只用全局 `fetch` 和自写的 SSE 切分。API key 从构造参数进入，不读环境变量。`Request.cacheBreakpoints` 里的 `"head"` 让 Anthropic 在 system 最后一个 block 和 tools 最后一个上打 `cache_control: {type:"ephemeral"}`；`{afterMessage:n}` 打在该消息最后一个 content block 上。OpenAI 兼容请求体不写任何 cache 字段。`ThinkingBlock` 在 Anthropic 侧原样回传（`redacted` 时为 `redacted_thinking`）；OpenAI 侧直接丢弃。

usage 合并后只在 `finish` 之前发一次。传输失败、协议截断、abort 都不从 `stream()` 抛出，分别以 `finish {kind:"error"}` 和 `finish {kind:"aborted"}` 结束，已经发出的 delta 保留。静态目录提供 `contextWindow` 和默认 `maxTokens`，`pricing` 一律 `null`。找不到型号时用 128000 / 4096，并设 `adapterDefaults.catalogFallback = true`。

流对象带 `responseTrace`（`responseId` / `responseModel`），这是线上报文里才出现的 L8 字段。适配器不写 Log，调用方拿去填 assistant entry。

## Alternatives considered

不用官方 SDK。issue 允许 SDK 只做 HTTP，但 SDK 自带的消息类型、错误类和重试会从传输层渗进词表，换一家 SDK 就得改上层。这一票的传输只有 POST 和 SSE，自己解析就够。contract 因此保持零外部 import，分层规则 1 不用为 SDK 开口子。

OpenAI 侧不保留 thinking。兼容协议的 assistant 消息没有 thinking 槽。塞进 `content` 会把思考原文发给模型，也破坏「编码不增删 token」。降级和 tool id 重写是 #13 的跨模型规整，不在适配器里做一半。

不远程拉模型目录。issue 明确不做。`prepare` 要的是调用前的 `contextWindow` 和默认 `maxTokens`，静态表够用。定价没有核对过的 source URL，保持 `null`，不填一个会进成本不等式的假数。

## Consequences

contract 除了类型还有纯函数 `collect`。它只 import 同目录文件，规则 1 仍然成立。OpenAI 请求会丢掉 thinking，直到 #13。core 里的 message 仍是同形结构，本票不改 core；Log 那份 note 里「#3 合入后改为 `import type`」还没做。

`stream()` 的声明返回类型仍是 `AsyncIterable<StreamChunk>`。`responseTrace` 是运行时挂在迭代对象上的字段，类型检查不把它算进词表。截断时若还没收到 usage，会补一条四元组为 0 的 usage，以满足「usage 在 finish 之前」。`adapterDefaults` 至少含 `maxTokens`；兜底时多一个 `catalogFallback`。

不做重试，不加图片或文件块，不把 provider 接进 core。

## Testing

`pnpm check` 退出码为 0（lint、tsc、分层、README 三问、log manifest、vitest 49 项）。共享契约套件对两个适配器各跑：去掉 raw 后的 chunk 序列一致、usage 在 finish 前且 finish 之后没有 chunk、截断不抛并保留已收到的 text delta、第二个 chunk 之后 abort 以 `finish {kind:"aborted"}` 结束。Anthropic 编码只在 system 末块、tools 末个、`messages[3]` 末块打 `cache_control`；OpenAI 请求体没有 cache 键，且不含 thinking 原文。`prepare` 对五条已知型号返回窗口和 `maxTokens: 8192`、`pricing: null`，未知型号带 `catalogFallback`。`collect` 组装文本、thinking 和 tool，解析失败时 `input` 为 `null`。测试用注入的 `fetchImpl` 回放 fixture，不打真网。
