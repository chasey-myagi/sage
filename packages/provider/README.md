# @sage/provider

本仓自己的模型请求词表，以及 Anthropic Messages、OpenAI 兼容 chat 两个适配器。适配器只翻译调用方给出的请求，不决定缓存断点。

## Model Experience

### What the model sees

本包决定线上格式（system/tools/messages 的编码）与断点位置的翻译。

### Token effect

规整会少发思考块：OpenAI 丢掉全部，Anthropic 丢掉没有 signature 的。tool id 重写不改正文。其余块的字节不变。

### KV cache effect

断点只按 `Request.cacheBreakpoints` 打，适配器不自作主张。丢掉 thinking 或重写 tool id 会改变发出去的消息字节；换 provider 发生在阶段边界，那一轮前缀本来就要重建。`Request.toolChoice` 可选 `"auto"` 或 `"none"`。没写时请求体不带 `tool_choice`。`"none"` 在 openai-chat 上是 `"none"`，在 anthropic-messages 上是 `{ "type": "none" }`；`"auto"` 同样映射。这个字段不改 system、tools、messages 的字节。

## 第三方 OpenAI 兼容端点

DeepSeek、百炼（DashScope）、Kimi（Moonshot）和 OpenAI 共用 `OpenAIAdapter`，只换 `baseUrl`。预设表在 `resolvePreset`：`openai`、`deepseek`、`dashscope`、`moonshot` 走 `openai-chat`，`anthropic` 走 `anthropic-messages`。

这些兼容端点的缓存是服务端自动前缀缓存。请求体没有 cache 字段，`cacheBreakpoints` 在这里无操作。

usage 按三种形状映射，`raw` 原样保留，`inputCacheCreation` 恒为 0（这些端点的报文不报缓存写入）：

- DeepSeek：有 `prompt_cache_hit_tokens` 时，`inputCacheRead` 取命中，`inputOther` 取 `prompt_cache_miss_tokens`；缺 miss 时用 `prompt_tokens - hit`。前缀缓存以 64 token 为一块，只对完全相同的前缀生效。目录里的单价是峰时价（工作日 01:00–04:00 与 06:00–10:00 UTC），非峰时半价。`decideEpoch` 只用读写倍率，半价不改变开不开新阶段。`bench report` 若把 token 换算成花费，要按实际时段，本仓现在不做这套换算。流式 `delta.reasoning_content` 收成没有 signature 的 thinking 块，写入 Log；下一轮编码时 `normalizeForApi` 把它剥掉，请求体不回传。
- OpenAI、DashScope、Moonshot：有 `prompt_tokens_details.cached_tokens` 时，命中进 `inputCacheRead`，其余 `prompt_tokens` 进 `inputOther`。
- 两种字段都没有：`inputOther = prompt_tokens`，`inputCacheRead = 0`。

目录 `Pricing.currency` 可选，缺省 USD。百炼和 Kimi 标 `CNY`，价格不换算成美元。缓存读写比率只除四个单价，不看币种。

## 传输错误重试

流还没开始时，下面这些失败会再试，最多 2 次。第一次等 1 秒，第二次等 4 秒。构造参数 `retry: { attempts, baseMs }` 改次数和基数，缺省是 2 和 1000。第 n 次重试等待 `baseMs * 4^(n-1)`。

会重试：

- `fetch` 抛错，包括连接被拒、连接重置、DNS 失败
- HTTP 429
- HTTP 5xx
- 2xx 但没有响应体

不会重试：

- 其余 4xx（例如 400）
- 调用方 abort
- 响应体已经开始之后的断开、截断、解析失败

响应体一开始就不再重试。前面可能已经吐出一段文本，再请求一次会把同一段输出生成两遍，调用方对不上。半截输出留在这一轮的 finish error 里。流开始之后的传输断开不在本包重试，同一请求的整轮重发由 `@sage/core` 负责。

finish error 上的 `transport: true` 标出传输失败：`fetch` 抛错、读响应体抛错（SSE 体或非 2xx 的错误体）、响应在完成前结束（2xx 没有 body，或 SSE 体在收到结束信号前正常关闭，消息为 `stream ended before completion`）。读完错误体的 HTTP 状态和流内 error 事件不打标。core 只凭这个标记决定是否整轮重发。

流内 error 事件是服务端在 200 响应里明确报错：Anthropic 的 `event: error`，OpenAI 兼容接口里带 `error` 对象的帧。adapter 把它解码成 `finish{kind:"error"}`，`error.message` 取服务端文本（例如 `Overloaded`），`error.raw` 是整个事件对象，不打 `transport`，之后的帧不再产出块。服务端已经给出结论，不是连接意外断开，所以 core 不整轮重发。

重试次数和最后一次失败原因写在这一轮的 usage 上：`details: { retries, reason }`。没重试就不写这个字段。会话本来会把 usage 和 finish 抄进 assistant 条目的 `details`，Log 因此能看见，不新增条目种类。中间几次失败的响应体不进 Log。成功那次如果前面失败过，`reason` 是最后一次失败的消息，不是成功本身。

测试可以注入 `sleep`，不真等这 1 秒和 4 秒。

## 跨模型规整

`normalizeForApi(messages, api)` 是纯函数。两个适配器在编码前各调用一次，只改发出去的副本，不改 Log 里的消息。

- `openai-chat`：丢掉全部 thinking 块。`toolu_` 开头的 tool id 改写成 `call_` 加上该 id 的 sha256 前 24 位十六进制，配对的 tool_result id 一起改。已经是别的形状的 id 不动。
- `anthropic-messages`：丢掉没有 signature 的 thinking 块。有 signature 的 thinking、文本和 tool id 原样保留。
