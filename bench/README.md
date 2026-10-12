# 基准

真机对照一组任务上的两套上下文策略。离线回放只回答「省了多少 token、前缀稳不稳、被省略的块后来有没有被引用」，见 [`packages/plugins/README.md`](../packages/plugins/README.md) 的「离线回放」。下一步动作是否一致、任务成不成，只在这里看。

**不进 CI。** 全套要取仓库、调用模型、跑测试，耗时和费用都由跑的人承担。本仓不提供 SWE-bench 官方 docker 评测环境。

## 准备环境

- Node.js 22。`corepack` 必须可用：Node 25 起不再自带 corepack，要么把 Node 22 放到 `PATH` 前面，要么先 `npm i -g corepack`，跑之前确认 `corepack pnpm` 能用。`ts-mini.json` 里 vuejs/core 的四条，`setupCommand` 和 `testCommand` 都调用 `corepack pnpm`。`ts-mini-2.json` 还要 corepack 能取到 `yarn@1.22.22`（zod、react-router-11160）和 `pnpm@9.9.0`（react-hook-form-12505），命令里写死了这两个版本。两份任务集都是在 Node 22.20.0、npm 10.9.3、corepack 0.34.0 上验证的。zx-1107 的锁文件只有 darwin-arm64 的 `@swc/core`、`esbuild` 平台条目，npm 11 的 `npm ci` 判它不同步，所以这条的 `setupCommand` 用 `npm install --no-save`：版本仍取锁文件，锁文件不改写；它另在 Node 25.9.0、npm 11.12.1 上验证过
- `git`
- 目标仓库能跑通的测试工具。Verified 任务的 `testCommand` 是 `python -m pytest …`，Python、依赖和被测仓库的构建由跑的人在执行测试的环境里准备好。`ts-mini.json` 和 `ts-mini-2.json` 的依赖由每条任务的 `setupCommand` 安装
- 模型 key。`--api-key-env` 只替换读哪个变量，选哪家仍由 `--provider` 或下面的自动顺序决定。没写 `--provider` 时，`DEEPSEEK_API_KEY`、`DASHSCOPE_API_KEY`、`MOONSHOT_API_KEY`、`OPENAI_API_KEY`、`ANTHROPIC_API_KEY` 里有一把即可。指定了 `--provider` 就只认那一把。没有对应的 key 时 `bench run` 直接退出 2，不会去取仓库

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

`testPatch`：unified diff。agent 结束、session 已复制、并且 `git status --porcelain`（排除 `.sage/`）非空之后才应用。从 `diff --git` 头取出路径，base 上有的文件执行 `git checkout <baseCommit> -- <file>`，base 上没有的文件执行 `rm -f`，然后 `git apply`。`git apply` 失败记 `error`，`reason` 为 `test-patch-failed`，不跑 `testCommand`。agent 运行期间工作区里没有这份隐藏测试。

## 任务集

| 文件 | 内容 |
|---|---|
| `bench/tasks/swe-verified-mini-50.json` | Hugging Face datasets-server 上 `princeton-nlp/SWE-bench_Verified` 的 `test` split 前 50 行 |
| `bench/tasks/mini.json` | 上面这一份的第一条，用来冒烟 |
| `bench/tasks/ts-mini.json` | Multi-SWE-bench（ByteDance-Seed）ts/js 切片里已在本机验证的 10 条。6 个仓库：vuejs/core ×4、darkreader ×2、dayjs、express、axios、github-readme-stats |
| `bench/tasks/ts-mini-2.json` | Multi-SWE-RL（ByteDance-Seed）里已在本机验证的 10 条，与上一份零重叠。7 个仓库：zx、commander.js ×2、redux、zod、react-router ×2、async ×2、react-hook-form |

数据许可：Multi-SWE-bench 与 Multi-SWE-RL 的数据集卡片声明 CC0，ByteDance 对数据集保留其拥有的知识产权。任务文件里嵌入的 issue 文本（`problem`）与 test patch（`testPatch`）取自上游仓库，使用时还受各仓库自己的许可约束。下表是入库任务文件涉及的仓库，许可证取自 GitHub API 的 `license.spdx_id`（2026-10-11）：

| 仓库 | 许可证 | 任务集 |
|---|---|---|
| vuejs/core | MIT | `ts-mini` |
| darkreader/darkreader | MIT | `ts-mini` |
| iamkun/dayjs | MIT | `ts-mini` |
| expressjs/express | MIT | `ts-mini` |
| axios/axios | MIT | `ts-mini` |
| anuraghazra/github-readme-stats | MIT | `ts-mini` |
| google/zx | Apache-2.0 | `ts-mini-2` |
| tj/commander.js | MIT | `ts-mini-2` |
| reduxjs/redux | MIT | `ts-mini-2` |
| colinhacks/zod | MIT | `ts-mini-2` |
| remix-run/react-router | MIT | `ts-mini-2` |
| caolan/async | MIT | `ts-mini-2` |
| react-hook-form/react-hook-form | MIT | `ts-mini-2` |
| astropy/astropy | BSD-3-Clause | `swe-verified-mini-50`、`mini` |
| django/django | BSD-3-Clause | `swe-verified-mini-50` |

入库的 JSON 是事实来源。上游数据集的 URL 没有钉修订，重新拉取只用于更新任务集，不用来复现已有结果。

重新拉取：

```sh
pnpm bench:fetch
pnpm bench:fetch-ts
```

两个脚本都零依赖。`bench:fetch` 把 `FAIL_TO_PASS` 收成 `python -m pytest <tests>`，`timeoutSec` 固定 1800。`--fixture <file>` 读本地 JSON，测试不打网络。

`bench:fetch-ts` 不走 datasets-server（这两个数据集的 `/rows` 会报 schema 错误），直接读 `https://huggingface.co/datasets/<dataset>/resolve/main/<path>/<org>__<repo>_dataset.jsonl`。每个仓库在脚本里声明自己的来源：

| 任务集 | dataset | path |
|---|---|---|
| `ts-mini` | `ByteDance-Seed/Multi-SWE-bench` | `ts` 或 `js` |
| `ts-mini-2` | `ByteDance-Seed/Multi-SWE-RL` | `data_20240601_20250331/ts` 或 `data_20240601_20250331/js`；react-hook-form 在 `data_20250401_20250631/ts` |

`data_20250401_20250631` 这一期的行没有 `instance_id`，id 按 `<org>__<repo>-<number>` 拼出来。Multi-SWE-RL 是 RL 训练集，没有人工校验，只收本机逐条验证过的 id。

`--set ts-mini|ts-mini-2` 选任务集，默认 `ts-mini`，默认输出到 `bench/tasks/<set>.json`。`--out <file>` 改输出路径，`--only id1,id2` 只取这几条。脚本按任务集里写死的 id 筛出任务：`problem` 是已解决 issue 的标题加正文，`testPatch` 是数据集里的 `test_patch`，`testCommand` 只跑这些 spec。个别实例的 base 用的包管理器或 lockfile 与同仓库其他条目不同，脚本按实例覆盖 setup / run。`--fixture <dir>` 改读 `<dir>/<org>__<repo>_dataset.jsonl`，测试不打网络。用仓库里的 fixture 跑，两个任务集的输出分别与 `ts-mini.json`、`ts-mini-2.json` 逐字节相同。

```sh
pnpm bench:fetch-ts --set ts-mini-2
```

## 怎么跑

从仓库根目录，先 `pnpm build`（bin 跑的是打包产物 `apps/sage/dist/cli.js`，每个任务的子进程 `sage -p` 也是）：

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

可选 `--shadow-scorer rule|model|jev`。写了就原样放进每个任务的 `sage -p` argv，没写就不传。别的名字在开跑之前退出，并打印用法。影子只记不生效：档次、冻结和预算仍由原来的编译器决定。跑完对账用 `sage stats <session.jsonl>`，不在 `results.json` 里另记这个旗标。要逐轮看 token、缓存命中、档位分布、阶段内翻档和阶段决策，用 `sage stats --turns <out>/sessions/<taskId>.jsonl`。真机 `bench run --shadow-scorer model` 不进 CI。

可选 `--scorer rule|model|jev` 和 `--scorer-apply entry|rebuild`。写了就原样透传，没写就不传。别的值在开跑之前退出，并打印用法。`results.json` 顶层 `scorerApply` 记下 `entry` 或 `rebuild`；没写旗标时是 `rebuild`。`entry` 要配 `--scorer model` 或 `--scorer jev`，否则每个任务里的 sage 以退出码 2 结束。对照是 `bench run --policy compiler --scorer model --scorer-apply entry`。真机对照不进 CI。

默认还给每个任务加上 `--tool-network none`。`--tool-sandbox none|full` 是同一个开关，后写的覆盖先写的。要让 bash 子进程上网，写 `--tool-network full` 或 `--tool-sandbox full`。`results.json` 顶层 `toolNetwork` 记下这次的值。断网只作用于工具子进程；父进程里的 provider 请求不受影响。评测必须默认断网：clone 只留 baseCommit 之后，agent 仍会 `curl` GitHub 上的上游提交，把修复抄进工作区，隐藏测试因此通过，这组结果作废。

`none` 同时打开文件系统隔离。每次运行用一个根目录，真实路径是 `<tmpdir 的 realpath>/sage-bench-<runId>/<taskId>/`。同一轮的任务都在这个 `runId` 下面，目录名是任务 id。一个任务结束就删掉它的目录，整轮结束再删根目录。进程中途被杀掉时，根目录会留在临时目录里。下次 `bench run` 在创建新根之前，把临时根下所有名字以 `sage-bench-` 开头的目录打到 stderr，每行一条 `warning: leftover bench workspace <path>; remove it manually`，不自动删。

agent 能读到什么：darwin 上，默认沙箱允许读本次运行根 `<tmpdir 的 realpath>/sage-bench-<runId>`，也允许读临时目录里的普通文件，拒绝读同一临时根下其他 `sage-bench-*` 运行根（包括上一轮没删掉的 clone）。任务顺序执行，上一条结束就删掉它的目录，同一运行根下没有还在的兄弟任务。因此 `cd <任务目录的绝对路径>` 可以穿过运行根。linux 的 `unshare -n` 做不到文件隔离，这一点写在 [`packages/tools/README.md`](../packages/tools/README.md)。`--tool-network full` 关掉网络限制，也关掉这层文件隔离，这时不设置 `allowRoot`。

没有新的命令行旗标。`fsIsolate` 打开时，bench 把运行根的 realpath 放进传给 sage 的环境变量 `SAGE_SANDBOX_ALLOW_ROOT`。

TS/JS 任务集（第二批把 `--tasks` 换成 `bench/tasks/ts-mini-2.json`）：

```sh
node apps/sage/bin/sage.mjs bench run \
  --tasks bench/tasks/ts-mini.json \
  --policy passthrough \
  --provider deepseek \
  --model <模型 id> \
  --out bench/runs/<时间戳>-ts-passthrough
```

通过 OpenAI 兼容的第三方网关跑时，加 `--base-url` 和 `--api-key-env`，两者原样透传给每个任务的 `sage -p`。开跑前检查的是 `--api-key-env` 指定的变量，不是预设的默认变量：

```sh
GATEWAY_API_KEY=<key> node apps/sage/bin/sage.mjs bench run \
  --tasks bench/tasks/ts-mini.json \
  --policy compiler \
  --provider moonshot \
  --model <模型 id> \
  --base-url <网关基址> \
  --api-key-env GATEWAY_API_KEY \
  --out bench/runs/<时间戳>-ts-compiler
```

工作区不能带着 `baseCommit` 之后的历史。agent 会跑 `git log --all`、`git show <sha>`、`git log --grep` 去翻上游是怎么改的；全量 clone 再 checkout 会把后来的修复提交、分支和标签留在仓库里，模型能把答案抄出来，通过率就失真了。

每个任务：

1. 在 `<tmpdir>/sage-bench-<runId>/<taskId>/` 里 `git init`，`git remote add origin <url>`，`git fetch --depth 1 --no-tags origin <baseCommit>`，`git checkout -q FETCH_HEAD`，再 `git remote remove origin`。fetch 失败记 `error`，`reason` 为 `clone-failed`。起 agent 之前，`git rev-list --all --count` 必须是 1，`git tag` 和 `git remote` 必须为空；否则记 `error`，`reason` 为 `clone-unclean`，不起 agent，也不跑 `setupCommand`。任务结束删除该目录；全部任务结束后删除 `sage-bench-<runId>`
2. 有 `setupCommand` 时，在该目录 `sh -c` 执行，时限 `timeoutSec` 秒。退出码非 0 记 `error`，`reason` 为 `setup-failed`，不起 agent，不跑测试
3. 在该目录执行 `sage -p <problem>`。argv 里有 `--max-turns <n>`，没写旗标时 n 是 80。argv 里有 `--tool-network none`，除非这次 run 写了 `--tool-network full`。写了 `--budget` 时，argv 里同时有 `--budget <tokens>`。时限 `timeoutSec` 秒
4. 复制 session。这一步在应用 `testPatch` 之前，session 里没有隐藏测试
5. sage 超时记 `error`，`reason` 为 `timeout`
6. 会话最后一条 `terminal` 的 `outcome` 为 `error` 时记 `error`，`reason` 为 `session-error`；为 `aborted` 时 `reason` 为 `session-aborted`。不看 diff，不应用 `testPatch`，不跑测试，工作区有改动也一样。`max_turns` 结束的会话照常往下判。`session-error` 的结果行另带 `providerError`，见下文结果目录
7. `git status --porcelain`（排除 sage 的运行时目录 `.sage/`）为空则记 `empty-patch`，不应用 `testPatch`，不跑测试
8. 有 `testPatch` 时，把 diff 涉及的文件还原到 `baseCommit`（base 没有的文件删掉），再 `git apply`。失败记 `error`，`reason` 为 `test-patch-failed`，不跑测试
9. 执行 `testCommand`，同样受 `timeoutSec` 限制。退出码 0 记 `resolved`，否则记 `error`，`reason` 为 `tests-failed`
10. fetch 失败记 `clone-failed`。工作区不是恰好一个提交，或还留着 tag、remote，记 `clone-unclean`。sage 启动失败、diff 失败记 `sage-failed` 或 `diff-failed`

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

`ts-mini-2.json` 的 install / test 秒数（2026-09-24，同一套 Node、npm、corepack，缓存是温的）。zx 和 redux-4139 的测试 import 的是构建产物而不是 `src`，所以 `testCommand` 先 build（zx 用 esbuild 约 3s，redux 用 rollup 约 5s），表里的 test 秒数含这一步。zx 的 `setupCommand` 也 build 一次，`build/` 在 gitignore 里，setup 之后 `git status` 仍然干净。多个 yarn1 安装并发会损坏共享缓存，zod 与 react-router-11160 要串行跑，或者每条任务用独立的 `YARN_CACHE_FOLDER`。

| id | install | test@base | test@fix |
|---|---|---|---|
| google__zx-1107 | 12.7s | 8.7s | 8.3s |
| tj__commander.js-1720 | 6.5s | 3.1s | 1.2s |
| tj__commander.js-1502 | 10.5s | 5.2s | 1.9s |
| reduxjs__redux-4139 | 8.2s | 10.1s | 5.7s |
| colinhacks__zod-3427 | 6.6s | 2.5s | 1.4s |
| remix-run__react-router-11786 | 22.4s | 3.8s | 2.2s |
| remix-run__react-router-11160 | 5.5s | 2.5s | 0.8s |
| caolan__async-1790 | 63.8s | 5.1s | 2.3s |
| caolan__async-1628 | 36.8s | 3.0s | 0.9s |
| react-hook-form__react-hook-form-12505 | 42.5s | 2.6s | 1.2s |

结果目录：

```text
<out>/results.json
<out>/sessions/<taskId>.jsonl
```

`results.json` 有 `policy`、`model`、`provider`、`budget`、`baseUrl`、`apiKeyEnv`、`toolNetwork`、`maxTurns`、`scorerApply`，以及每个任务的 `id`、`status`、可选 `reason`、`health`、`stoppedEarly`。`reason` 为 `session-error` 的行另有 `providerError`：会话以 provider 错误结束时是 `{ status, transport }`，`status` 是 HTTP 状态码（HTTP 以下的失败为 `null`），`transport` 为 true 表示 fetch 抛错或响应体没读完；会话因 provider 以外的原因出错时为 `null`。错误文本不写进这份文件。compiler 组的 `budget` 是 `--budget` 的正整数；没写这个旗标，或策略是 passthrough，则为 `null`。`toolNetwork` 默认 `none`，写了 `--tool-network full` 时为 `full`。`maxTurns` 是 `--max-turns` 的正整数；没写这个旗标时为 80，两种策略都记。`provider` 是 `--provider` 的值；没写时是按自动选择顺序实际选中的那家，子进程拿到同一份环境变量，选的是同一家。`baseUrl` 和 `apiKeyEnv` 是这两个旗标的值，没写时为 `null`；只记变量名，不记 key。`health` 是该 session 的 `runHealth`。问题陈述和块原文不写进这份文件。跑完的目录不要提交。

## 报表怎么读

```sh
node apps/sage/bin/sage.mjs bench report <runA> <runB>
```

stdout 是一份 JSON：`a`、`b`、`delta`。`a` 和 `b` 各带 `budget`、`baseUrl` 和 `provider`，与该目录 `results.json` 的顶层同名字段相同，文件里没有时为 `null`。`delta` 是 B 减 A，不含这三项。没有 key 也能打报表。

| 字段 | 口径 |
|---|---|
| `resolvedRate` | `status === "resolved"` 的任务数 / 任务总数 |
| `emptyPatchRate` | `empty-patch` 的任务数 / 任务总数 |
| `errorRate` | `error` 的任务数 / 任务总数。含 `session-error` 与 `session-aborted`：会话没有正常结束的任务不进 `emptyPatchRate`，也不进 `resolvedRate`。对照实验要剔除故障格时，按 `reason` 从两边的 `results.json` 去掉这些任务再比 |
| `tokensPerResolved` | 全部任务 `health.tokens.total` 之和 / resolved 数。失败任务的 token 算在分子里。resolved 为 0 时是 `null` |
| `cacheHitRatio` | 各任务 `health.cacheHitRatio` 的算术平均，`null` 不进入平均。单个任务的这个数是 Σ cacheRead / Σ (cacheRead + inputOther + cacheCreation) |
| `expandRate` | 各任务 `health.expandRate` 的算术平均，`null` 不进入平均。单个任务的这个数是最后一份清单的 `budget.expands / budget.omitted`，`omitted` 为 0 时该任务是 `null` |

任务数为 0 时三率是 `null`。某一侧的指标是 `null` 时，对应的 `delta` 也是 `null`。

`tokensPerResolved` 下降表示每个解决样本花的 token 更少。`resolvedRate` 上升表示 B 解决得更多。`expandRate` 上升表示被省略的块里，后来又被 `expand` 取回的比例更高。
