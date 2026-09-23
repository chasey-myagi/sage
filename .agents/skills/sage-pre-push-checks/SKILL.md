---
name: sage-pre-push-checks
description: 在 sage 仓库提交、推送、开 PR 或声称「检查通过」之前使用——选对测试、跑闸门、写 commit 与 PR 描述、处理失败。Use before committing, pushing, opening a PR, or claiming checks pass in sage.
---

# 推送前检查

`pnpm check` 是唯一闸门，全量约 20 秒，推送前总是跑全量。本 skill 管的是迭代时跑什么、失败怎么处理、提交物怎么写。

## 1. 看清要推什么

```sh
git status --short --branch
git fetch origin
git log --oneline origin/main..HEAD
git diff origin/main...HEAD --stat
```

范围超出「一件事」就拆分支。确认没有混进无关文件（bench 结果、`.sage/` 会话、临时脚本）。

## 2. 迭代时跑最窄的证据

- 行为改动：先跑拥有这个行为的测试文件，`pnpm vitest run <file>`；找不到就用 `pnpm vitest related <src 文件> --run`，再检查选中的测试是否真的覆盖了改动。
- 新增或修改 Log 条目：`pnpm gen:log-manifest`（它读 `tsc -b` 的产物，先跑一次 `pnpm check:types`）。
- 包 README：`pnpm check:readme`。
- 分层：`pnpm check:layers`。
- CLI 行为：走 `runCli` 的测试（`apps/sage/test/`），不只测内部函数。

## 3. 推送前跑全量

```sh
pnpm check
```

报告里只写真正跑过的命令和结果。不跑就不说「应该能过」。

## 4. 失败怎么办

- 测试红了修根因。不改断言、不删用例、不加 `skip`、不调低阈值。
- 若失败是因为有意改变了行为，旧测试钉的是被淘汰的行为：在同一个 PR 里改测试，并在 PR 描述与 Agent Note 里逐条说明为什么。
- 怀疑是环境问题（沙箱、网络、平台）：记下命令、失败的用例和差异，证明其余部分通过，再说明为什么 CI 会不同。不带着红灯推送。

## 5. commit

标题 `<类型>(<包>[,<包>]): <中文一句话>`，类型取 `feat` / `fix` / `refactor` / `test` / `docs` / `chore`。正文中文短句：问题是什么、改了什么、刻意没做什么、验证结果。一个 PR 合并前压成一个提交。

## 6. PR 描述

```
## 做了什么
<一两句>

## 为什么
<问题与证据>

## 没做什么
<刻意排除的范围>

## 验证
<跑过的命令与结果；红灯测试名>
```

非平凡改动在 `docs/notes/implemented/` 附 Agent Note，PR 描述链接它。

## 7. 推送之后

```sh
git rev-parse HEAD origin/$(git branch --show-current)
gh pr checks
```

CI 在跑就报「进行中」，失败先看日志再判断是分支的问题还是环境的问题。改写历史只用 `--force-with-lease`，不用裸 `--force`。
