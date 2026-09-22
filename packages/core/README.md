# @sage/core

内核：只追加的 Log，以及每次调用模型前的 View seam。

## Model Experience

### What the model sees

head + messages 全由 ViewProvider 决定，内核只断言与落盘。

### Token effect

内核不增删任何 token。

### KV cache effect

内核保证阶段内 head 逐字节不变、断点原样透传，`request_header` 是事后证据。换模型不改 head。

## model_change

阶段边界如果视图带了 `epochStart.model`，并且和当前型号不同，内核在 `view_epoch` 之前追加 `model_change`。payload 是 `{ from, to, reason }`。reason 是 `route:` 加上清单 `epochDecision.reason` 里 `;route:` 之后的那段。然后切到 `adapters[provider]`，用新型号再 `prepare` 一次。随后的 `request_header` 用这次 prepare 的 provider、model 和 adapterDefaults。

找不到该 provider 的适配器时，追加 `terminal`，outcome 为 `error`，不发请求。型号相同，或者没有 `epochStart.model`，不写 `model_change`。

`RunSummary.modelChanges` 等于这次 run 里追加的 `model_change` 条数。

构造可以传 `adapters`（provider 到适配器），也可以继续传单个 `adapter`。单个适配器记在起始型号的 provider 名下。

## 测试

无 key 是默认：测试进程会剥掉 `*_API_KEY`，并用录制的响应回放，不打真网。
`pnpm test:update` 重写期望产物，PR 里要逐行看这些 diff。
破坏前缀稳定会炸快照：head 变了，下一轮就不再是 `<last>` 加增量，而是整段重印。
