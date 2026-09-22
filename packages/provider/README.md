# @sage/provider

本仓自己的模型请求词表，以及 Anthropic Messages、OpenAI 兼容 chat 两个适配器。适配器只翻译调用方给出的请求，不决定缓存断点。

## Model Experience

### What the model sees

本包决定线上格式（system/tools/messages 的编码）与断点位置的翻译。

### Token effect

规整会少发思考块：OpenAI 丢掉全部，Anthropic 丢掉没有 signature 的。tool id 重写不改正文。其余块的字节不变。

### KV cache effect

断点只按 `Request.cacheBreakpoints` 打，适配器不自作主张。丢掉 thinking 或重写 tool id 会改变发出去的消息字节；换 provider 发生在阶段边界，那一轮前缀本来就要重建。

## 跨模型规整

`normalizeForApi(messages, api)` 是纯函数。两个适配器在编码前各调用一次，只改发出去的副本，不改 Log 里的消息。

- `openai-chat`：丢掉全部 thinking 块。`toolu_` 开头的 tool id 改写成 `call_` 加上该 id 的 sha256 前 24 位十六进制，配对的 tool_result id 一起改。已经是别的形状的 id 不动。
- `anthropic-messages`：丢掉没有 signature 的 thinking 块。有 signature 的 thinking、文本和 tool id 原样保留。
