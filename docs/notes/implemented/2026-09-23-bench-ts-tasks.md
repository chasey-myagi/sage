# TS/JS 基准任务集

## Problem

现有任务集是 SWE-bench Verified，测试命令是 pytest。这台机器没有那套 Python 评测环境，`bench run` 一条都跑不完。Multi-SWE-bench 的 ts/js 切片里已经挑出 10 条并在 Node 22 上验证过，但隐藏测试被塞进 `testCommand` 的 base64，runner 也没有「checkout 之后、起 agent 之前」的安装步骤。

## Decision

任务增加两个可选字段。`setupCommand` 在 checkout 之后、起 agent 之前于 clone 目录执行 `sh -c`，时限沿用 `timeoutSec`。退出码非 0（含这条命令自己超时）记 `error` / `setup-failed`，不起 agent，不跑测试，并计入 `errorRate`。`testPatch` 是 unified diff。agent 结束、session 复制完成、且 `git status --porcelain` 非空之后，才把 diff 头里的路径从 `baseCommit` 还原（base 没有的文件 `rm -f`），再 `git apply`，然后跑 `testCommand`。apply 失败记 `test-patch-failed`，不跑测试。agent 运行期间工作区里没有这份隐藏测试。

`bench/tasks/ts-mini.json` 是这 10 条。`testCommand` 只剩测试命令。`bench/tasks/fetch-multi-swe-ts.mjs` 零依赖，直接下 Hugging Face `resolve/main/<lang>/*.jsonl`（datasets-server 对这个数据集报 schema 错误）。`pnpm bench:fetch-ts` 调用它。`--fixture` 读本地 jsonl，测试不打网。Node 版本不进任务文件，README 写明要 Node 22，并且 vue 四条要 `corepack pnpm`。

## Alternatives considered

把 test patch 继续嵌在 `testCommand` 里，runner 不用改。10 条里最长的一条命令大约 8KB，能跑，但任务文件读不了，以后每接一个语言切片都要再嵌一遍。

`setup-failed` 单独记成 skipped，对比两套策略时不进分母。issue 要求的状态是 `error`。安装失败和上下文策略无关，但它仍然是这次 run 没跑完的任务，分母保持整份列表。

给 setup 和测试各加一个超时字段，或在任务里写 Node 版本。实测 setup 不超过 35 秒，测试不超过 6 秒，`timeoutSec` 1800 装得下。版本放进任务会让 runner 去管 PATH，而 corepack 是否存在取决于调用者已经选好的 Node。

## Consequences

没有 `setupCommand` / `testPatch` 的 Python 任务走原来的路径。session 在应用 `testPatch` 之前复制，Log 里不记 setup 命令、测试 patch 和测试输出。

`setup-failed` 与 `test-patch-failed` 都进 `errorRate`。两套策略如果在同一次环境故障上各记一条 error，delta 看不出策略差别。那是环境问题，不是这个字段的口径问题。

离线 fixture 是这 10 条原始 jsonl 行裁掉 runner 不读的字段（vue 四条的完整行各有大约 1.5MB 运行日志）。脚本输出与入库的 `ts-mini.json` 逐字节相同，也与「从 base64 拆开」的那份素材在 problem、setup、patch、测试命令上一致。

本票没有模型 key，不跑真机 `bench run`。issue 里「10 条全部走完、没有 clone-failed / setup-failed」仍要有 key、并且 PATH 上是 Node 22 的人来补。

## Testing

红灯先写了两条：`setupCommand` 退出非 0 时期望 `setup-failed` 且不起 agent；agent 改过的测试文件在跑测试前被 base 加 `testPatch` 盖掉。实现前前者收到 `started === ["setup-bad"]`，后者收到 `agent-test` / `agent-extra`。

绿灯是这两条，加上 `--fixture` 的拉取结果与 `ts-mini.json` 逐字节相同。`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、vitest 304 项）。测试不 clone GitHub，不打模型。
