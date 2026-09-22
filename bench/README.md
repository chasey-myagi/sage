# 基准

真机对照一组任务上的两套上下文策略。离线回放只回答「省了多少 token、前缀稳不稳、被省略的块后来有没有被引用」，见 [`packages/plugins/README.md`](../packages/plugins/README.md) 的「离线回放」。下一步动作是否一致、任务成不成，只在这里看。

**不进 CI。** 全套要 clone 仓库、调用模型、跑测试，耗时和费用都由跑的人承担。本仓不提供 SWE-bench 官方 docker 评测环境。

## 准备环境

- Node.js 22 或更新
- `git`
- 目标仓库能跑通的测试工具。Verified 任务的 `testCommand` 是 `python -m pytest …`，Python、依赖和被测仓库的构建由跑的人在执行测试的环境里准备好
- `ANTHROPIC_API_KEY` 或 `OPENAI_API_KEY`。指定了 `--provider` 就只认那一把。没有对应的 key 时 `bench run` 直接退出 2，不会 clone

任务文件是 JSON 数组，每项：

```json
{
  "id": "owner__repo-n",
  "repo": "owner/repo",
  "baseCommit": "<git sha>",
  "problem": "问题陈述",
  "testCommand": "python -m pytest tests/test_x.py::test_y",
  "timeoutSec": 1800
}
```

`repo` 不带协议时，按 `https://github.com/<repo>.git` clone。

## 任务集

| 文件 | 内容 |
|---|---|
| `bench/tasks/swe-verified-mini-50.json` | Hugging Face datasets-server 上 `princeton-nlp/SWE-bench_Verified` 的 `test` split 前 50 行 |
| `bench/tasks/mini.json` | 上面这一份的第一条，用来冒烟 |

重新拉取：

```sh
pnpm bench:fetch
```

脚本零依赖。`FAIL_TO_PASS` 收成 `python -m pytest <tests>`，`timeoutSec` 固定 1800。`--fixture <file>` 读本地 JSON，测试不打网络。

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

`--policy passthrough` 给每个任务加上 `--passthrough`。`compiler` 用 CLI 默认编译器（预算 40000，规则打分）。可选 `--provider anthropic|openai`。

每个任务：

1. clone 到临时目录，`checkout baseCommit`
2. 在该目录执行 `sage -p <problem>`，时限 `timeoutSec` 秒
3. `git status --porcelain` 为空则记 `empty-patch`，不跑测试
4. 否则执行 `testCommand`，同样受 `timeoutSec` 限制。退出码 0 记 `resolved`，否则记 `error`
5. clone、sage 启动失败、diff 失败、超时，都记 `error`，`reason` 是 `clone-failed`、`sage-failed`、`diff-failed`、`tests-failed` 或 `timeout`

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
