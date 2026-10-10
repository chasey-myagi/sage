# bench 判空排除 sage 的运行时目录

## Problem

bench 用 `git status --porcelain` 判断任务有没有产出改动，但子进程 sage 会在工作区写 `.sage/sessions/<id>.jsonl`；bash 输出超过 30 000 字节时还会写 `.sage/tool-output/<toolUseId>.txt`。这两类文件都不是模型的改动，却让 `diffEmpty` 永远返回 false：`empty-patch` 在真机上恒不触发，`stoppedEarly` 的「diff 非空」条件恒真（#131）。

## Decision

`diffEmpty` 用 pathspec `:(exclude).sage` 排除整个 `.sage/`。`.sage` 这个目录名在 `@sage/tools` 定义为 `SAGE_DIR_NAME`，find / grep 的跳过列表、bash 的溢出路径、apps 的会话目录 `SESSION_LOG_SUBDIR` 与 bench 的排除规则都引用它。

## Alternatives considered

只排除 `.sage/sessions`：第一版这样做，评审用真实 git 仓库证明溢出文件仍会让判空失败。逐个排除子目录要求每新增一种运行时文件就记得改 bench，而 find / grep 早已把整个 `.sage` 当作非用户内容跳过，两处口径应当一致。在 bench 里先删掉 `.sage/` 再判空：会让 `copySession` 与判空之间多一个顺序约束，且删除动作本身不是只读检查。

## Consequences

模型如果在 bench 里只往 `.sage/` 写文件（例如新增子任务预设），任务记为 `empty-patch`。这符合「`.sage/` 不是用户的工作」这一口径。plugins 的子任务预设路径（`plugins/src/subtask/presets.ts`）仍写字面量 `.sage`：plugins 不 import tools，要共用只能下沉到 core，留给后续。

## Testing

`apps/sage/test/regressions/issue-131.test.ts` 通过 `runCli` 跑 `bench run`，用真实 git 工作区与生产 `diffEmpty`：只有会话 Log、只有溢出的工具输出两种情形都应为 `empty-patch`，改已跟踪文件、新增未跟踪文件应为有改动。修复前前两种被判 `resolved`；第二种在只排除 sessions 的版本上仍失败，先红后绿。
