# bench 工作区隔离

## Problem

2026-09-23 真机上，历史清理和断网都已经生效，`vuejs__core-11625` 仍用 bash 执行 `ls` 扫 `$TMPDIR`，读到被中途杀掉的旧 bench clone，再去打开隔壁工作区里「更新版本」的源文件。临时根目录对 bash 是敞开的，一次运行的工作区也和其他 `sage-bench-*` 堆在同一层。

## Decision

`createTools` 接收 `sandbox: { network, fsIsolate }`。`network` 默认 `"full"`。没写 `fsIsolate` 时，`"none"` 为 `true`，`"full"` 为 `false`。darwin 的 `sandbox-exec` profile 在网络规则之后先写 `(deny file-read* (subpath "<tmpRoot>"))`，再写 `(allow file-read* (subpath "<cwd>"))`。后写的规则覆盖先写的。`tmpRoot` 与 `cwd` 都取 realpath，macOS 上 `/var` 变成 `/private/var`。bash 因此读得到自己的工作目录，读不到临时根目录里的其他路径。

linux 的 `unshare -n` 不能限制文件。`fsIsolate` 不改变这条 argv，也不让 `network: "none"` 创建失败。`network: "full"` 且 `fsIsolate` 为 true 时，只有 darwin 做得到。

`bench run` 的工作目录是 `<realpath(tmpdir)>/sage-bench-<runId>/<taskId>/`。同一轮任务共用一个 `runId`。任务结束删除该任务目录，整轮结束删除根目录。创建根目录之前，把临时根下名字以 `sage-bench-` 开头的目录打到 stderr：`warning: leftover bench workspace <path>; remove it manually`。不自动删除。

`--tool-sandbox none|full` 与 `--tool-network` 是同一个开关，后写的覆盖先写的。传给每个任务的 argv 仍是 `--tool-network`。`none` 同时打开 `fsIsolate`。`results.json` 仍只记 `toolNetwork`。`fsIsolate` 不进 bash 的 `details`，也不进 Log。

## Alternatives considered

不在启动时删掉所有 `sage-bench-*`。那些目录可能是另一次还在跑的 bench，或是别人留下要自己看的现场。只警告，不代删。

不把文件隔离做不到当成 linux 上 `network: "none"` 的失败。`unshare -n` 从来就不能限制文件；因此退出会让 linux 上的默认评测全部起不来。缺口写在 README 里。

不只拒绝当前 `runId` 根目录。上一轮留下的孤儿在临时根的另一层，只封住这一轮的根挡不住 `ls $TMPDIR`。

不用容器或 chroot。这一票不加依赖，darwin 已有的 `sandbox-exec` 够用。

## Consequences

darwin 上默认的 `none` 会让 bash 读不到临时根里除当前 cwd 以外的路径，包括同一轮的其他任务目录和以前留下的 clone。`--tool-network full` 同时关掉网络限制和这层文件隔离。linux 上文件仍然可读。孤儿目录要人手删。子进程 argv 继续使用 `--tool-network`，已有的断言不用改口。`details` 仍是 `{ network: "none" }`。

## Testing

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、391 个测试）。darwin 上按与实现相同的 profile 真跑 `sandbox-exec`：`ls` 临时根退出码 1，stderr 为 `Operation not permitted`，stdout 为空；`cat` 隔壁文件退出码 1，stdout 为空，不含文件正文；`cat` 工作目录内文件退出码 0，stdout 为该文件正文。`bench run` 两条假任务的目录是同一个 `sage-bench-<runId>` 下的 `one` 与 `two`，任务一结束后目录已不在，整轮结束后根目录不存在。预先放好的 `sage-bench-orphan-*` 仍在，stderr 含 `warning: leftover bench workspace <path>; remove it manually`，且不含本次 run 根的目录名。除这次 sandbox-exec 验证外，测试不打网，不跑真机 bench。
