# tools 与 plugins（编译器、子任务之外）的注释与文档

## Problem

`packages/tools/src` 与 `packages/plugins/src` 里 `compiler/`、`subtask/` 之外的文件，文件头仍是逐 PR 追加的行为清单：`bash.ts` 39 行、`model.ts` 与 `offline.ts` 各 32 行、`jev.ts` 29 行。多数导出符号没有 JSDoc。`bash.ts` 有裸数字 `124`、`2000`，`find.ts` 与 `ls.ts` 各写一份 `500`，`grep.ts` 与 `find.ts` 各写一份跳过目录表，claim 的默认优先级与离线回放的默认预算也是裸数字。`createBash(cwd, execCwd, ...)` 两个路径参数的区别只能读实现才知道。测试里还有 `scenario 1 tool declarations` 这类编号式用例名。

## Decision

只改注释、命名与常量，行为与模型可见文本不变。

- 文件头按 G7 重写为职责、保证、`Intentionally not in the Log:`，全部 ≤ 25 行：`bash.ts` 39 → 24、`model.ts` 32 → 13、`offline.ts` 32 → 15、`jev.ts` 29 → 11、`tools/index.ts` 23 → 15。原文件头的命题逐条搬到实现它的函数、类型或字段的 JSDoc（`createBash`、`sandboxProfile`、`toolSandboxSupport`、`modelScorer`、`jevScorer`、`TurnDiff` 各字段等），没有删掉事实。
- 范围内每个导出符号都有 JSDoc；只为测试存在的选项标 `Test seam.`。
- 具名常量：`TIMEOUT_EXIT_CODE`（124，与 GNU `timeout(1)` 一致）、`SIGNALED_EXIT_CODE`、`UNSHARE_PROBE_TIMEOUT_MS`、`FIT_MAX_ATTEMPTS`、`LIST_MAX_ENTRIES`（find 与 ls 共用）、`SKIPPED_DIR_NAMES`（grep、find 与 rg 参数共用）、`RG_NO_MATCH_EXIT_CODE`、四个 claim 的默认优先级、`REPLAY_DEFAULT_BUDGET`；离线回放的 `keepRecent` 与 `expectedTurns` 默认值改引用编译器的 `DEFAULT_KEEP_RECENT`、`DEFAULT_EXPECTED_TURNS`。工具 schema 与打分指令里的默认值改用模板引用常量，字节不变。
- `execCwd` 改名 `realCwd`，JSDoc 写明：`cwd` 是调用方写法，模型看到的 spill 路径由它拼出；`realCwd` 是 realpath，子进程在那里运行，也是默认的沙箱放行根，因为 `sandbox-exec` 按解析后的路径匹配。
- 用例名：13 个 describe 与 1 个 it 改成描述行为的标题，测试体不动。

## Alternatives considered

把文件头的行为细节搬进包 README：README 已经写了同样的事实，面向使用者；函数上的 JSDoc 面向改代码的人，离代码更近，不会和实现分开漂移。所以细节放 JSDoc，README 不重复加。

## Consequences

`packages/tools/test/regressions/issue-51.test.ts` 的文件名与测试体注释里仍有 issue 编号：回归文件名是仓库约定，测试注释不在这次范围。

## Testing

`pnpm check` 退出码 0（87 个文件、419 个测试），测试断言一行未改。临时导出七个工具的 `def` 做 JSON 比对，改动前后逐字节相同；打分指令由 `issue-69.test.ts` 的快照钉住。
