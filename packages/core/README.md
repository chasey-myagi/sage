# @sage/core

内核：只追加的 Log，以及每次调用模型前的 View seam。

## Model Experience

### What the model sees

head + messages 全由 ViewProvider 决定，内核只断言与落盘。

### Token effect

内核不增删任何 token。

### KV cache effect

内核保证阶段内 head 逐字节不变、断点原样透传，`request_header` 是事后证据。

## 测试

无 key 是默认：测试进程会剥掉 `*_API_KEY`，并用录制的响应回放，不打真网。
`pnpm test:update` 重写期望产物，PR 里要逐行看这些 diff。
破坏前缀稳定会炸快照：head 变了，下一轮就不再是 `<last>` 加增量，而是整段重印。
