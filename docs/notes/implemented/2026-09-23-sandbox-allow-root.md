# 沙箱放行本次运行根目录

## Problem

2026-09-23，#82 合入后的真机上，10 条任务有 28 次工具错误是 `sh: line 0: cd: /private/var/folders/…/T/sage-bench-<runId>/<taskId>: Not a directory`。模型习惯写 `cd <绝对路径> && …`。两条 vue 任务因此拖到超时。

profile 先 `(deny file-read* (regex #"^<tmpRoot>/sage-bench-"))`，再 `(allow file-read* (subpath "<cwd>"))`。绝对路径要穿过父目录 `sage-bench-<runId>/`，这一层的元数据读取被正则拒了。cwd 内的相对路径不受影响。

## Decision

沙箱选项增加 `allowRoot`，默认是 cwd 的 realpath。darwin 的 profile 仍是先写拒绝前缀的正则，再写 `(allow file-read* (subpath "<allowRoot 的 realpath>"))`。后写的规则覆盖先写的。

`bench run` 在 `fsIsolate` 打开时，把 `sandbox.allowRoot` 设成这次运行根的 realpath。子进程 sage 通过环境变量 `SAGE_SANDBOX_ALLOW_ROOT` 收到它。没有新的 CLI 旗标。`--tool-network full` 关掉文件隔离，这时不设置 `allowRoot`。

同一轮任务顺序执行，一条结束就删掉它的目录，运行根下没有还在的兄弟任务。放行整棵运行根不让 bash 读到另一次运行的 `sage-bench-*`。

## Alternatives considered

不加 `--allow-root`。路径属于这一次运行，不是用户每次要选的开关。旗标进 argv 也会进帮助文本，和「只放行当前运行根」这件内部的事不匹配。

不放行整个临时根。测试框架和 pnpm 需要读临时目录里的普通文件，但隔壁 `sage-bench-*` 必须继续拒绝。

不改成只允许父目录的 metadata。issue 里的复现已经表明，把 allow 的 subpath 换成运行根，`cd` 就能通过，隔壁目录仍是 `Operation not permitted`。

不把文件隔离做不到当成 linux 的失败。`unshare -n` 仍然只断网。

## Consequences

darwin 上，默认的 `none` 让 bash 读得到本次 `sage-bench-<runId>`，读不到其他 `sage-bench-*`。`cd <任务目录的绝对路径>` 可以穿过运行根。临时目录里其余文件仍可读。`--tool-network full` 同时关掉网络限制和这层文件隔离。linux 上文件仍然可读。`allowRoot` 和 `SAGE_SANDBOX_ALLOW_ROOT` 不进 Log，也不进 bash 的 `details`。

## Testing

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、83 个文件、403 个测试）。darwin 上真跑 `sandbox-exec`：cwd 为 `<tmpRoot>/sage-bench-x/task`、`allowRoot` 为 `<tmpRoot>/sage-bench-x` 时，`cd <绝对 cwd> && cat f` 成功；`cat <tmpRoot>/sage-bench-y/f` 被拒，正文含 `Operation not permitted`，不含隔壁文件内容。`bench run` 传给 sandbox 的 `allowRoot` 等于该任务目录的 realpath 的父目录。`SAGE_SANDBOX_ALLOW_ROOT` 进入子进程后，darwin profile 的 allow subpath 是这个路径的 realpath。除这次沙箱验证外，测试不打网，不跑真机 bench。
