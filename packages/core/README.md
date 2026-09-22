# @sage/core

内核：只追加的 Log，以及每次调用模型前的 View seam。

## Model Experience

### What the model sees

head + messages 全由 ViewProvider 决定，内核只断言与落盘。

### Token effect

内核不增删任何 token。

### KV cache effect

内核保证阶段内 head 逐字节不变、断点原样透传，`request_header` 是事后证据。
