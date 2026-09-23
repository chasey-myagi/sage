# 基准

真机对照一组任务上的两套上下文策略。离线回放只回答「省了多少 token、前缀稳不稳、被省略的块后来有没有被引用」，见 [`packages/plugins/README.md`](../packages/plugins/README.md) 的「离线回放」。下一步动作是否一致、任务成不成，只在这里看。

**不进 CI。** 全套要取仓库、调用模型、跑测试，耗时和费用都由跑的人承担。本仓不提供 SWE-bench 官方 docker 评测环境。

## 准备环境

- Node.js 22。系统自带的 Node 25 没有 corepack。`ts-mini.json` 里 vuejs/core 的四条，`setupCommand` 和 `testCommand` 都调用 `corepack pnpm`，跑之前要把 Node 22 放到 `PATH` 前面，并确认 `corepack pnpm` 能用。这 10 条是在 Node 22.20.0、npm 10.9.3、corepack 0.34.0 上验证的
- `git`
- 目标仓库能跑通的测试工具。Verified 任务的 `testCommand` 是 `python -m pytest …`，Python、依赖和被测仓库的构建由跑的人在执行测试的环境里准备好。`ts-mini.json` 的依赖由每条任务的 `setupCommand` 安装
- 模型 key。没写 `--provider` 时，`DEEPSEEK_API_KEY`、`DASHSCOPE_API_KEY`、`MOONSHOT_API_KEY`、`OPENAI_API_KEY`、`ANTHROPIC_API_KEY` 里有一把即可。指定了 `--provider` 就只认那一把。没有对应的 key 时 `bench run` 直接退出 2，不会去取仓库

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

`setupCommand` 和 `testPatch` 可以不写。Verified 的 Python 任务没有这两项。`repo` 不带协议时，url 是 `https://github.com/<repo>.git`。取下来的工作区只有 `baseCommit` 这一个提交。

`setupCommand`：取出 `baseCommit` 之后、起 agent 之前，在工作目录里用 `sh -c` 执行，时限是 `timeoutSec`。退出码非 0，包括这条命令自己超时，记 `error`，`reason` 为 `setup-failed`，不起 agent，不跑测试。`timeout` 这个 reason 只用于 agent 和 `testCommand`。`setup-failed` 记进 `errorRate`：环境没准备好，和上下文策略无关，但分母仍是整份任务列表。

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
  --budget 6000 \
  --out bench/runs/<时间戳>-compiler
```

`--policy passthrough` 给每个任务加上 `--passthrough`。`compiler` 用 CLI 默认编译器（预算 40000，规则打分）。可选 `--provider`，取值是预设名，例如 `deepseek`。可选 `--budget <tokens>`，透传给每个任务的 `sage -p`（passthrough 也透传，sage 会忽略并在 stderr 提示一句）。对照实验请显式压预算，默认 40000 在短任务上不会触发编译器。

可选 `--max-turns <n>`，必须是正整数，否则退出 2，还没读任务文件。每个任务的 argv 里都有 `--max-turns`：写了旗标用那个数，没写则是 80。passthrough 也透传，不会忽略。`results.json` 顶层 `maxTurns` 是这个整数。

可选 `--shadow-scorer rule|model|jev`。写了就原样放进每个任务的 `sage -p` argv，没写就不传。别的名字在开跑之前退出，并打印用法。影子只记不生效：档次、冻结和预算仍由原来的编译器决定。跑完对账用 `sage stats <session.jsonl>`，不在 `results.json` 里另记这个旗标。真机 `bench run --shadow-scorer model` 不进 CI。

可选 `--scorer rule|model|jev` 和 `--scorer-apply entry|rebuild`。写了就原样透传，没写就不传。别的值在开跑之前退出，并打印用法。`results.json` 顶层 `scorerApply` 记下 `entry` 或 `rebuild`；没写旗标时是 `rebuild`。`entry` 要配 `--scorer model` 或 `--scorer jev`，否则每个任务里的 sage 以退出码 2 结束。对照是 `bench run --policy compiler --scorer model --scorer-apply entry`。真机对照不进 CI。

默认还给每个任务加上 `--tool-network none`。`--tool-sandbox none|full` 是同一个开关，后写的覆盖先写的。要让 bash 子进程上网，写 `--tool-network full` 或 `--tool-sandbox full`。`results.json` 顶层 `toolNetwork` 记下这次的值。断网只作用于工具子进程；父进程里的 provider 请求不受影响。评测必须默认断网：clone 只留 baseCommit 之后，agent 仍会 `curl` GitHub 上的上游提交，把修复抄进工作区，隐藏测试因此通过，这组结果作废。

`none` 同时打开文件系统隔离。每次运行用一个根目录，真实路径是 `<tmpdir 的 realpath>/sage-bench-<runId>/<taskId>/`。同一轮的任务都在这个 `runId` 下面，目录名是任务 id。一个任务结束就删掉它的目录，整轮结束再删根目录。进程中途被杀掉时，根目录会留在临时目录里。下次 `bench run` 在创建新根之前，把临时根下所有名字以 `sage-bench-` 开头的目录打到 stderr，每行一条 `warning: leftover bench workspace <path>; remove it manually`，不自动删。

agent 能读到什么：darwin 上，默认沙箱允许读当前任务目录，拒绝读临时根目录以及其他 `sage-bench-*`（包括上一轮没删掉的 clone 和同一轮里别的任务目录）。linux 的 `unshare -n` 做不到文件隔离，这一点写在 [`packages/tools/README.md`](../packages/tools/README.md)。`--tool-network full` 关掉网络限制，也关掉这层文件隔离。

TS/JS 这 10 条：

```sh
node apps/sage/bin/sage.mjs bench run \
  --tasks bench/tasks/ts-mini.json \
  --policy passthrough \
  --provider deepseek \
  --model <模型 id> \
  --out bench/runs/<时间戳>-ts-passthrough
```

工作区不能带着 `baseCommit` 之后的历史。agent 会跑 `git log --all`、`git show <sha>`、`git log --grep` 去翻上游是怎么改的；全量 clone 再 checkout 会把后来的修复提交、分支和标签留在仓库里，模型能把答案抄出来，通过率就失真了。

每个任务：

1. 在 `<tmpdir>/sage-bench-<runId>/<taskId>/` 里 `git init`，`git remote add origin <url>`，`git fetch --depth 1 --no-tags origin <baseCommit>`，`git checkout -q FETCH_HEAD`，再 `git remote remove origin`。fetch 失败记 `error`，`reason` 为 `clone-failed`。起 agent 之前，`git rev-list --all --count` 必须是 1，`git tag` 和 `git remote` 必须为空；否则记 `error`，`reason` 为 `clone-unclean`，不起 agent，也不跑 `setupCommand`。任务结束删除该目录；全部任务结束后删除 `sage-bench-<runId>`
2. 有 `setupCommand` 时，在该目录 `sh -c` 执行，时限 `timeoutSec` 秒。退出码非 0 记 `error`，`reason` 为 `setup-failed`，不起 agent，不跑测试
3. 在该目录执行 `sage -p <problem>`。argv 里有 `--max-turns <n>`，没写旗标时 n 是 80。argv 里有 `--tool-network none`，除非这次 run 写了 `--tool-network full`。写了 `--budget` 时，argv 里同时有 `--budget <tokens>`。时限 `timeoutSec` 秒
4. 复制 session。这一步在应用 `testPatch` 之前，session 里没有隐藏测试
5. sage 超时记 `error`，`reason` 为 `timeout`
6. `git status --porcelain` 为空则记 `empty-patch`，不应用 `testPatch`，不跑测试
7. 有 `testPatch` 时，把 diff 涉及的文件还原到 `baseCommit`（base 没有的文件删掉），再 `git apply`。失败记 `error`，`reason` 为 `test-patch-failed`，不跑测试
8. 执行 `testCommand`，同样受 `timeoutSec` 限制。退出码 0 记 `resolved`，否则记 `error`，`reason` 为 `tests-failed`
9. fetch 失败记 `clone-failed`。工作区不是恰好一个提交，或还留着 tag、remote，记 `clone-unclean`。sage 启动失败、diff 失败记 `sage-failed` 或 `diff-failed`

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

`results.json` 有 `policy`、`model`、`budget`、`toolNetwork`、`maxTurns`、`scorerApply`，以及每个任务的 `id`、`status`、可选 `reason`、`health`。compiler 组的 `budget` 是 `--budget` 的正整数；没写这个旗标，或策略是 passthrough，则为 `null`。`toolNetwork` 默认 `none`，写了 `--tool-network full` 时为 `full`。`maxTurns` 是 `--max-turns` 的正整数；没写这个旗标时为 80，两种策略都记。`health` 是该 session 的 `runHealth`。问题陈述和块原文不写进这份文件。跑完的目录不要提交。

## 报表怎么读

```sh
node apps/sage/bin/sage.mjs bench report <runA> <runB>
```

stdout 是一份 JSON：`a`、`b`、`delta`。`a` 和 `b` 各带 `budget`，与该目录 `results.json` 的顶层 `budget` 相同。`delta` 是 B 减 A，不含 `budget`。没有 key 也能打报表。

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
