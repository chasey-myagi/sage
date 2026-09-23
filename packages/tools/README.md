# @sage/tools

第一方工具：`read` / `write` / `edit` / `bash` / `grep` / `find` / `ls`。

## Model Experience

### What the model sees

七个 JSON Schema。越出工作区的路径不抛异常：`read` / `write` / `edit` / `grep` / `find` / `ls` 返回 `isError` 的 tool_result，正文是 `[error] Path is outside the workspace: …`，不读不写。`accesses()` 对这种路径返回空列表，同样不抛。工具错误不能结束会话。`read` 默认只给前 2000 行，超长时末行是 `[truncated: showing lines 1-2000 of N; call read with offset=… to continue]`。`bash` 结果以 `[bash exit code N]` 开头；输出超过 30000 字节时保留头 20000 与尾 5000，中间一行是 `[… omitted N bytes; full output saved to <path>; read it with read(path, offset=…) …]`。`bash` 的网络策略不出现在 schema 里，也不写进工具结果正文。断网时模型只看到命令自己的报错，例如 curl 解析主机失败。`grep` 每条是 `file:line: text`。超过 300 字符的行只留匹配附近的 300 字符：匹配起点在前 300 字符里就留开头，否则留匹配点前 150 与后 150；被截掉的一侧加 `…`，行末加 `[+N chars]`。`edit` 在 0 次或多次匹配时返回错误并写明次数。

### Token effect

`read` 上限 2000 行且 262144 字节，不返回半行。`grep` 默认 200 条，每条行文本最多 300 字符。`find` / `ls` 默认 500 条。`bash` 预览连同通知行一起算在 30000 字节以内（头 20000、尾 5000）。`timeout_ms` 默认 120000，不占 token。网络策略本身不占 token。

### KV cache effect

无。工具结果追加在消息尾部；schema 在 head 上，同一次运行内不变。网络策略不改 schema。

## 网络

`createTools({ sandbox: { network } })` 的 `network` 取 `"full"` 或 `"none"`，默认 `"full"`。`"none"` 只包住 bash 拉起的子进程：darwin 用 `sandbox-exec`，外网（含 DNS）拒绝，localhost 仍可用；linux 在 `unshare -n` 可用时用它，不可用则创建工具时失败，不静默改回 `"full"`；其他平台同样失败。父进程里的 provider 请求不经过这层包裹。

评测必须用 `"none"`。仓库只剩 baseCommit 时，agent 会 `curl` GitHub 把上游修复抄下来，隐藏测试因此通过，结果作废。

`network: "none"` 记在该次 bash 的 `details` 上。内核把它写到用户消息的 `details.toolResults`，模型看不见。`"full"` 不写这条。`fsIsolate` 不进 details。

## 文件系统

`createTools({ sandbox: { network, fsIsolate, allowRoot } })`。`network` 仍是 `"full"` 或 `"none"`，默认 `"full"`。没写 `fsIsolate` 时，`"none"` 把它当成 `true`，`"full"` 当成 `false`。没写 `allowRoot` 时，它等于 cwd 的 realpath。

agent 在 darwin、`fsIsolate` 打开时，bash 子进程能读 `allowRoot` 这棵目录，也能读临时目录里的普通文件，但读不到同一临时根下的其他 bench 工作区。没写 `allowRoot` 时，它等于 `cwd` 的 realpath。profile 先写 `(deny file-read* (regex #"^<tmpRoot 的 realpath，元字符已转义>/<前缀>"))`，再写 `(allow file-read* (subpath "<allowRoot 的 realpath>"))`。后写的规则覆盖先写的。`tmpRoot` 和前缀都是选项；没写前缀时用 `sage-bench-`，bench 传入自己的前缀。路径里的 `.`、`+` 等正则元字符会转义。macOS 上 `/var` 会先解析成 `/private/var`。临时根以外的系统路径仍走 `(allow default)`，二进制和库读得到。`$TMPDIR` 里不以该前缀开头的文件，包括测试框架写回的临时文件，可以读。

`cd <绝对路径>` 要读取路径上每一层目录。允许范围只有任务目录时，父目录 `sage-bench-<runId>` 落在拒绝规则里，`cd` 会报 `Not a directory`。把 `allowRoot` 设成这次运行的根目录，任务目录的绝对路径就能 `cd` 进去。隔壁另一次运行的 `sage-bench-*` 仍被拒绝。没有对应的 CLI 旗标。bench 把运行根的 realpath 放进环境变量 `SAGE_SANDBOX_ALLOW_ROOT`，sage 启动时读它并传给沙箱。

linux 的 `unshare -n` 只切网络命名空间，不能限制文件。这时 `fsIsolate` 不会改 argv，也不会让 `network: "none"` 创建失败。`network: "full"` 再要求 `fsIsolate` 时，只有 darwin 做得到，其他平台创建工具即失败。`"full"` 且不隔离文件时，不包沙箱，文件系统不限制。
