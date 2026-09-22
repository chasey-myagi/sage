# @sage/provider

本仓自己的模型请求词表，以及 Anthropic Messages、OpenAI 兼容 chat 两个适配器。适配器只翻译调用方给出的请求，不决定缓存断点。

## Model Experience

### What the model sees

本包决定线上格式（system/tools/messages 的编码）与断点位置的翻译。

### Token effect

编码不增删 token，除 OpenAI 侧丢弃 thinking 块。

### KV cache effect

断点只按 `Request.cacheBreakpoints` 打，适配器不自作主张。
