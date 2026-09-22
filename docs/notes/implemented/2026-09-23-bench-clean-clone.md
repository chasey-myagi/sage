# bench 只保留 baseCommit 一个提交

## Problem

2026-09-23 三方对照里，sage 跑 `anuraghazra__github-readme-stats-2228`，会话里的 bash 依次是 `git log --all --oneline --grep="progress bar" -i`、`git show c0d7493`，然后 `git show c0d7493:src/cards/wakatime-card.js` 拷到临时文件再 diff，输出 `SRC EXACT MATCH`。`c0d7493` 就是上游修这个 issue 的提交。sage 和 Cursor grok 在这条任务上的源码 diff 逐字节相同，连注释都一样。

扫这次 `bench run` 的全部 20 个会话（passthrough / compiler / compiler-8000），14 个用过 `git log --all`、`git show <sha>`、`git log --grep`。之前报的 resolvedRate 0.9 不可信。

`bench.ts` 用 `git clone` 全量克隆再 `checkout baseCommit`。工作区里仍留着这次提交之后的全部提交、分支和标签。

## Decision

准备工作目录改为：`git init`，`git remote add origin <url>`，`git fetch --depth 1 --no-tags origin <baseCommit>`，`git checkout -q FETCH_HEAD`，`git remote remove origin`。不带协议的 `repo` 仍是 `https://github.com/<repo>.git`。GitHub 允许按 SHA fetch。`--no-tags` 关掉 git 默认的 tag 跟随：指向已下载对象的 tag 本来会被带下来，验收要求工作区里没有 tag。

fetch 任一步失败记 `error` / `clone-failed`。取完后断言 `git rev-list --all --count` 为 1、`git tag` 为空、`git remote` 为空；不满足记 `error` / `clone-unclean`，不跑 `setupCommand`，不起 agent。

`setupCommand`、`testPatch`、`--budget` 的路径不动。浅提交仍是原来的 SHA，`git checkout <baseCommit> -- <file>` 和 `git apply` 继续用这个对象。

## Alternatives considered

全量 clone 之后删掉 baseCommit 以后的分支和 tag。对象还在包里，`git show <sha>` 和 `git cat-file` 照样读得到上游修复。这次真机走的就是这条路。

`git clone --depth 1` 默认分支再 checkout。baseCommit 往往不是分支尖，depth 1 的尖就是后来的修复，checkout 要么失败，要么工作区里根本没有要改的那一版。

把历史压成一个新提交。SHA 变了，`testPatch` 按 `baseCommit` 还原文件会对不上。

不删 remote，只浅抓。agent 还能 `git fetch` 把后面的提交拉下来。origin 必须拿掉。

## Consequences

工作区是 detached HEAD 的浅仓库，本地只有 `baseCommit` 这一个提交，没有 remote，没有 tag。`git log --all` 翻不到未来历史。

runner 不拦网络。agent 若从题目或 `package.json` 里知道上游地址，仍可能自己加 remote 再 fetch。这一票只保证准备好的工作区里没有这段历史。

某个 host 拒绝按 SHA fetch 时，该任务记 `clone-failed`，不起 agent。issue 里的真机 10 条由跑的人确认没有这个 reason。

`clone-unclean` 进 `errorRate`，和 `clone-failed` 一样。两套策略如果在同一次取仓库失败上各记一条 error，delta 看不出策略差别。

## Testing

红灯先写：假仓库在 baseCommit 之后再提交一次并打上 `v-future`，`bench run` 用生产的 prepare。实现前工作区是 `count: 2`，`git log --all` 有两条 SHA，remote 是 `origin`，tag 是 `v-future`。

绿灯是这条，加上 fetch 不到该 SHA 时记 `clone-failed` 且不起 agent。原有的 setup / testPatch / budget 测试没改断言。`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、vitest 321 项）。测试用 `file://` 本地仓库，不 clone GitHub，不打模型，不跑真机 bench。
