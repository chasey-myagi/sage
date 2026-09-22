# @sage/core

内核：只追加的 Log。View seam 不在本票。

## Model Experience

### What the model sees

无。Log 不进模型请求；进模型的是 #5 的 View。

### Token effect

无。

### KV cache effect

无。Log 里的 `request_header` 与 `view_epoch` 是缓存命中与否的事后证据。
