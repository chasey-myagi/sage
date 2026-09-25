# 第二批 TS/JS 任务集 ts-mini-2

## Problem

`ts-mini.json` 只有 6 个仓库，vue 占 4 条，真机 passthrough 9/10，区分度不够。第二批 10 条已在开发机上逐条验证：base 上打隐藏测试失败，再打 gold patch 通过，setup 之后 `git status` 为空；passthrough 跑出 6/10。但任务文件和生成脚本都还在临时目录里，别人复现不了。

Multi-SWE-bench 的 ts/js 只有 9 个仓库，6 个第一批已经用了，剩下的 mui、svelte、insomnia 都太大。第二批取自同一组织的 `ByteDance-Seed/Multi-SWE-RL`。它字段相同，但按时间分目录，`data_20250401_20250631` 这一期的行没有 `instance_id`。现有拉取脚本只认一个 dataset、按语言分路径、按 `instance_id` 筛选，也没法给单个实例换包管理器。

## Decision

`bench/tasks/ts-mini-2.json` 按验证过的那份原样入库。`fetch-multi-swe-ts.mjs` 扩成一张仓库表加两个任务集：

- 每个仓库声明 `dataset` 和 `path`，URL 统一是 `https://huggingface.co/datasets/<dataset>/resolve/main/<path>/<org>__<repo>_dataset.jsonl`。
- id 取 `instance_id`，缺了就是 `<org>__<repo>-<number>`。筛选和输出都用这个 id。
- `OVERRIDES` 按实例覆盖 setup / run：react-router-11160 的 base 还在用 yarn1；react-hook-form-12505 没有 `packageManager` 字段，裸 `corepack pnpm` 会取到 corepack 0.34 起不来的版本，固定 `pnpm@9.9.0`；async-1790 的 lockfile 缺 watchify，改用 `npm install --no-package-lock`。
- `SETS` 里是 `ts-mini` 和 `ts-mini-2` 两份 id 列表，各带默认输出文件。`--set` 选其中一份，默认 `ts-mini`。参数解析改用 `node:util` 的 `parseArgs`，保留 `--fixture`、`--out`、`--only`。
- spec 文件的判定合成一条正则，加上排除 `/support/`、`__snapshots__`、`deno/`。第二批的生成器多认 `*-spec.js` / `*-test.js` 和 `tests/*.js`，少认 `test/unit/**`；两边取并集后，用 fixture 跑出的两份输出都与入库文件逐字节相同。
- zx、redux 的测试 import 构建产物，build 写进 `testCommand` 前面，不加 `buildCommand` 字段，runner 不动。

Multi-SWE-RL 是 RL 训练集，没有人工校验，所以任务集只收本机验证过的 id。淘汰的候选（KaTeX、rhf-9662、trpc-5156、redux-3452）不进配置。

## Alternatives considered

为每个任务集各配一条 spec 正则。并集已经对两份输出都成立，分开只会多一层配置；以后哪批出现分歧再拆。

给任务加 `buildCommand` 字段。runner、README、结果口径都要跟着改，而 build 只有两条任务需要，几秒钟，放进 `testCommand` 就够了。

另写一个 `fetch-multi-swe-rl.mjs`。两边字段相同，差的只是来源和 id 兜底，拆成两个脚本会复制行转任务、spec 判定和 problem 拼接。

## Consequences

`zod` 与 `react-router-11160` 用 yarn1，多个 yarn1 安装并发会损坏共享缓存，跑的人要串行，或者给每条任务单独的 `YARN_CACHE_FOLDER`。corepack 必须能取到 `yarn@1.22.22` 和 `pnpm@9.9.0`。README 写明了这两点，以及第二批的 install / test 秒数。

离线 fixture 新增 7 个仓库的 jsonl，只留脚本读的字段（`instance_id`、`org`、`repo`、`number`、`base`、`resolved_issues`、`test_patch`），合计约 63KB。react-hook-form 那一行没有 `instance_id`，与数据集一致，字节对比测试因此也覆盖了 id 兜底。

以后加任务集只改 `SETS`、必要时补 `REPOS` / `OVERRIDES`，再放 fixture 行、加一条逐字节测试。

## Testing

红灯先行：`fetch-multi-swe-ts --set ts-mini-2 matches ts-mini-2.json byte for byte from a fixture` 与 `fetch-multi-swe-ts names a row without instance_id <org>__<repo>-<number>` 在改脚本之前都以 usage 错误失败（旧脚本不认 `--set`）。后者用一行手写的无 `instance_id` 数据，期望 id 是 `tj__commander.js-7`。

改完后这两条和原有的 `fetch-multi-swe-ts matches ts-mini.json byte for byte from a fixture` 都通过。`pnpm check` 全绿。测试只读 fixture，不打网络。本票没有重跑真机 fail→pass 验证，依据是 2026-09-24 的验证记录，秒数表也取自那次。
