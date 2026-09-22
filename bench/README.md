# 基准

真机对照一组任务上的两套上下文策略。离线回放只回答「省了多少 token、前缀稳不稳、被省略的块后来有没有被引用」，见 [`packages/plugins/README.md`](../packages/plugins/README.md) 的「离线回放」。下一步动作是否一致、任务成不成，只在这里看。

**不进 CI。** 全套要 clone 仓库、调用模型、跑测试，耗时和费用都由跑的人承担。本仓不提供 SWE-bench 官方 docker 评测环境。

## 准备环境

- Node.js 22。系统自带的 Node 25 没有 corepack。`ts-mini.json` 里 vuejs/core 的四条，`setupCommand` 和 `testCommand` 都调用 `corepack pnpm`，跑之前要把 Node 22 放到 `PATH` 前面，并确认 `corepack pnpm` 能用。这 10 条是在 Node 22.20.0、npm 10.9.3、corepack 0.34.0 上验证的
- `git`
- 目标仓库能跑通的测试工具。Verified 任务的 `testCommand` 是 `python -m pytest …`，Python、依赖和被测仓库的构建由跑的人在执行测试的环境里准备好。`ts-mini.json` 的依赖由每条任务的 `setupCommand` 安装
- 模型 key。没写 `--provider` 时，`DEEPSEEK_API_KEY`、`DASHSCOPE_API_KEY`、`MOONSHOT_API_KEY`、`OPENAI_API_KEY`、`ANTHROPIC_API_KEY` 里有一把即可。指定了 `--provider` 就只认那一把。没有对应的 key 时 `bench run` 直接退出 2，不会 clone

任务文件是 JSON 数组，每项：

```json
{
  "id": "owner__repo-n",
  "repo": "owner/repo",
  "baseCommit": "<git sha>",
  "problem": "问题陈述",
  "setupCommand": "npm ci",
  "testPatch": "diff --git a/test/x.js b/test/x.js\n...",
  "testCommand": "npx jest test/x.js",
  "timeoutSec": 1800
}
```

`setupCommand` 和 `testPatch` 可以不写。Verified 的 Python 任务没有这两项。`repo` 不带协议时，按 `https://github.com/<repo>.git` clone。

`setupCommand`：checkout 之后、起 agent 之前，在 clone 目录里用 `sh -c` 执行，时限是 `timeoutSec`。退出码非 0，包括这条命令自己超时，记 `error`，`reason` 为 `setup-failed`，不起 agent，不跑测试。`timeout` 这个 reason 只用于 agent 和 `testCommand`。`setup-failed` 记进 `errorRate`：环境没准备好，和上下文策略无关，但分母仍是整份任务列表。

`testPatch`：unified diff。agent 结束、session 已复制、并且 `git status --porcelain` 非空之后才应用。从 `diff --git` 头取出路径，base 上有的文件执行 `git checkout <baseCommit> -- <file>`，base 上没有的文件执行 `rm -f`，然后 `git apply`。`git apply` 失败记 `error`，`reason` 为 `test-patch-failed`，不跑 `testCommand`。agent 运行期间工作区里没有这份隐藏测试。

## 任务集

| 文件 | 内容 |
|---|---|
| `bench/tasks/swe-verified-mini-50.json` | Hugging Face datasets-server 上 `princeton-nlp/SWE-bench_Verified` 的 `test` split 前 50 行 |
| `bench/tasks/mini.json` | 上面这一份的第一条，用来冒烟 |
| `bench/tasks/ts-mini.json` | Multi-SWE-bench（ByteDance-Seed）ts/js 切片里已在本机验证的 10 条。6 个仓库：vuejs/core ×4、darkreader ×2、dayjs、express、axios、github-readme-stats |

重新拉取：

```sh
pnpm bench:fetch
pnpm bench:fetch-ts
```

两个脚本都零依赖。`bench:fetch` 把 `FAIL_TO_PASS` 收成 `python -m pytest <tests>`，`timeoutSec` 固定 1800。`--fixture <file>` 读本地 JSON，测试不打网络。

`bench:fetch-ts` 不走 datasets-server（这个数据集的 `/rows` 会报 schema 错误），直接读 `huggingface.co/datasets/ByteDance-Seed/Multi-SWE-bench/resolve/main/<lang>/*.jsonl`。按写死的 10 个 id 筛出任务：`problem` 是已解决 issue 的标题加正文，`testPatch` 是数据集里的 `test_patch`，`testCommand` 只跑这些 spec。`--fixture <dir>` 改读 `<dir>/<org>__<repo>_dataset.jsonl`，测试不打网络。用仓库里的 fixture 跑，输出与 `bench/tasks/ts-mini.json` 逐字节相同。

## 怎么跑

从仓库根目录：

```sh
node apps/sage/bin/sage.mjs bench run \
  --tasks bench/tasks/swe-verified-mini-50.json \
  --policy passthrough \
  --model <模型 id> \
  --out bench/runs/<时间戳>-passthrough

node apps/sage/bin/sage.mjs bench run \
  --tasks bench/tasks/swe-verified-mini-50.json \
  --policy compiler \
  --model <模型 id> \
  --out bench/runs/<时间戳>-compiler
```

`--policy passthrough` 给每个任务加上 `--passthrough`。`compiler` 用 CLI 默认编译器（预算 40000，规则打分）。可选 `--provider`，取值是预设名，例如 `deepseek`。

TS/JS 这 10 条：

```sh
node apps/sage/bin/sage.mjs bench run \
  --tasks bench/tasks/ts-mini.json \
  --policy passthrough \
  --provider deepseek \
  --model <模型 id> \
  --out bench/runs/<时间戳>-ts-passthrough
```

每个任务：

1. clone 到临时目录，`checkout baseCommit`
2. 有 `setupCommand` 时，在该目录 `sh -c` 执行，时限 `timeoutSec` 秒。退出码非 0 记 `error`，`reason` 为 `setup-failed`，不起 agent，不跑测试
3. 在该目录执行 `sage -p <problem>`，时限 `timeoutSec` 秒
4. 复制 session。这一步在应用 `testPatch` 之前，session 里没有隐藏测试
5. sage 超时记 `error`，`reason` 为 `timeout`
6. `git status --porcelain` 为空则记 `empty-patch`，不应用 `testPatch`，不跑测试
7. 有 `testPatch` 时，把 diff 涉及的文件还原到 `baseCommit`（base 没有的文件删掉），再 `git apply`。失败记 `error`，`reason` 为 `test-patch-failed`，不跑测试
8. 执行 `testCommand`，同样受 `timeoutSec` 限制。退出码 0 记 `resolved`，否则记 `error`，`reason` 为 `tests-failed`
9. clone、sage 启动失败、diff 失败也记 `error`，`reason` 是 `clone-failed`、`sage-failed` 或 `diff-failed`

`ts-mini.json` 的 install / test 秒数（2026-09-23，Node 22.20.0，npm 与 pnpm 缓存是温的）。`test@base` 是打上隐藏测试后失败那次，`test@fix` 是再打上 gold patch 后通过那次。dayjs 的 install 首轮是 31.5s，表内 7.2s 是温缓存。vue core 冷的 pnpm store 大约 14s，darkreader 冷的 `npm ci` 大约 20s。

| id | install | test@base | test@fix |
|---|---|---|---|
| vuejs__core-11813 | 15.7s | 2.4s | 1.1s |
| vuejs__core-10289 | 13.3s | 2.2s | 0.9s |
| vuejs__core-10874 | 10.0s | 5.1s | 2.4s |
| vuejs__core-11625 | 9.0s | 3.1s | 0.8s |
| darkreader__darkreader-7241 | 15.3s | 2.8s | 1.9s |
| darkreader__darkreader-6747 | 9.0s | 3.8s | 1.4s |
| iamkun__dayjs-2377 | 7.2s | 1.4s | 0.8s |
| expressjs__express-5555 | 33.5s | 0.7s | 0.4s |
| axios__axios-5661 | 16.2s | 1.7s | 0.3s |
| anuraghazra__github-readme-stats-2228 | 5.6s | 1.3s | 0.6s |

结果目录：

```text
<out>/results.json
<out>/sessions/<taskId>.jsonl
```

`results.json` 只有 `policy`、`model`，以及每个任务的 `id`、`status`、可选 `reason`、`health`。`health` 是该 session 的 `runHealth`。问题陈述和块原文不写进这份文件。跑完的目录不要提交。

## 报表怎么读

```sh
node apps/sage/bin/sage.mjs bench report <runA> <runB>
```

stdout 是一份 JSON：`a`、`b`、`delta`。`delta` 是 B 减 A。没有 key 也能打报表。

| 字段 | 口径 |
|---|---|
| `resolvedRate` | `status === "resolved"` 的任务数 / 任务总数 |
| `emptyPatchRate` | `empty-patch` 的任务数 / 任务总数 |
| `errorRate` | `error` 的任务数 / 任务总数 |
| `tokensPerResolved` | 全部任务 `health.tokens.total` 之和 / resolved 数。失败任务的 token 算在分子里。resolved 为 0 时是 `null` |
| `cacheHitRatio` | 各任务 `health.cacheHitRatio` 的算术平均，`null` 不进入平均。单个任务的这个数是 Σ cacheRead / Σ (cacheRead + inputOther + cacheCreation) |
| `expandRate` | 各任务 `health.expandRate` 的算术平均，`null` 不进入平均。单个任务的这个数是最后一份清单的 `budget.expands / budget.omitted`，`omitted` 为 0 时该任务是 `null` |

任务数为 0 时三率是 `null`。某一侧的指标是 `null` 时，对应的 `delta` 也是 `null`。

`tokensPerResolved` 下降表示每个解决样本花的 token 更少。`resolvedRate` 上升表示 B 解决得更多。`expandRate` 上升表示被省略的块里，后来又被 `expand` 取回的比例更高。
