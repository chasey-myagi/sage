# bash 工具网络策略

## Problem

2026-09-23，干净 clone、仓库只有 baseCommit 一个提交时，sage 跑 `anuraghazra__github-readme-stats-2228`。会话里的 bash 依次是：`git log --all` 只看到 1 个提交，然后 `curl` raw.githubusercontent.com 上的 `master/src/cards/wakatime-card.js`、`api.github.com` 的 commits 接口、`github.com` 上 `c0d7493c3f0c.patch`，再 curl 该提交的源文件，然后 edit。最终源码与上游逐字节相同，隐藏测试通过。本地历史清掉之后，它改上网找答案。评测结果因此无效。

## Decision

`createTools({ network })` 取 `"full"` 或 `"none"`，默认 `"full"`。`"none"` 只包住 bash 子进程：darwin 用 `sandbox-exec -p '(version 1)(allow default)(deny network*)(allow network* (local ip "localhost:*"))' sh -c`，外网和 DNS 拒绝，localhost 仍可用；linux 在 `unshare -n` 能跑通时用 `unshare -n sh -c`，否则创建工具时失败；其他平台同样失败。失败不降级回 `"full"`。`sage` 在 `--tool-network none` 且当前平台做不到时退出 2，stderr 一行说明。

`sage [-p]` 的 `--tool-network` 默认 `full`。`bench run` 默认 `none`，argv 里总是带上这个旗标，`--tool-network full` 才放开。`results.json` 顶层记 `toolNetwork`。

命令自己的 stdout/stderr 原样进工具结果，不断网失败另写一句。`network: "none"` 放在 execute 的 `details` 里。内核已有的位置是用户消息的 `details.toolResults`（按 toolUseId 对应那次 bash），不进模型可见的 tool_result 正文。`"full"` 不写这条。

## Alternatives considered

不用代理环境变量。子进程可以 `unset HTTPS_PROXY`，也可以在命令里写 `env -u`。Node 自带的 fetch（undici）默认不读 `HTTP_PROXY` / `HTTPS_PROXY`，设了也拦不住 `node -e` 里的 fetch，只可能影响会认代理的 curl。sandbox 拒绝的是网络本身，子进程改环境变量退不出去。

不在工具结果里追加「网络已被拒绝」。命令自己的退出码和报错已经是模型可见面；再包一层会让失败看起来像工具故障，而不是命令失败。

不把 `network` 写进 bash 的 schema。模型不需要知道策略开关；schema 一变，head 的 tools hash 也会变。

## Consequences

评测默认断网。放开必须显式 `--tool-network full`，并且会写进 `results.json`。父进程的 provider 请求不经过这层包裹。子任务拿到的是同一份 bash 工具，策略一起带上。文件系统仍然不隔离，这不是通用沙箱。

linux 上没有可用的 `unshare -n` 时，`sage --tool-network none` 启动即退出 2，`bench run` 的默认也会让每个任务的 sage 如此退出，不会悄悄带着网跑。`"full"` 不在 Log 里留 details；`"none"` 只出现在 `details.toolResults`，不在 assistant 的 tool_use 块上。argv 本身不进 Log。

`curl -s` 在这份 sandbox 下退出码是 6（CURLE_COULDNT_RESOLVE_HOST），stdout 和 stderr 都是空的。解析失败的正文要去掉 `-s` 才看得到。工具不改写这个输出。

## Testing

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、329 个测试）。darwin 上真跑 `sandbox-exec`：`curl -s --max-time 5 https://api.github.com/` 非 0 且为退出码 6，输出不含 GitHub API 正文；`node -e` 起本地 http 并自连成功。`network: "full"` 的假 exec 看到 argv 就是原命令。假 platform `win32` 与 linux 且 `unshare` 不可用时，`sage` 退出 2，错误文案分别点名平台和 `unshare -n`。`bench run` 默认 argv 含 `--tool-network none`，`results.json.toolNetwork` 为 `"none"`；`--tool-network full` 记下 `"full"`。Log 里该次 bash 的 `details.toolResults` 为 `{ network: "none" }`，消息 payload 不含这个字段。除 sandbox 那条外，测试不打网，不跑真机 bench。
