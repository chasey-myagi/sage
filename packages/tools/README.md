# @sage/tools

第一方工具：`read` / `write` / `edit` / `bash` / `grep` / `find` / `ls`。

## Model Experience

### What the model sees

七个 JSON Schema。`read` 默认只给前 2000 行，超长时末行是 `[truncated: showing lines 1-2000 of N; call read with offset=… to continue]`。`bash` 结果以 `[bash exit code N]` 开头；输出超过 30000 字节时保留头 20000 与尾 5000，中间一行是 `[… omitted N bytes; full output saved to <path>; read it with read(path, offset=…) …]`。`grep` 每条是 `file:line: text`。超过 300 字符的行只留匹配附近的 300 字符：匹配起点在前 300 字符里就留开头，否则留匹配点前 150 与后 150；被截掉的一侧加 `…`，行末加 `[+N chars]`。`edit` 在 0 次或多次匹配时返回错误并写明次数。

### Token effect

`read` 上限 2000 行且 262144 字节，不返回半行。`grep` 默认 200 条，每条行文本最多 300 字符。`find` / `ls` 默认 500 条。`bash` 预览连同通知行一起算在 30000 字节以内（头 20000、尾 5000）。`timeout_ms` 默认 120000，不占 token。

### KV cache effect

无。工具结果追加在消息尾部；schema 在 head 上，同一次运行内不变。
