# core 按 childRuns 统计子会话，不再认插件工具名

## Problem

`packages/core/src/health.ts` 按工具名 `spawn_subtask` / `spawn_subtasks` 识别子会话，并猜两种 details 形状（单个是 `details.usage`，批量是 `details.results[].usage`）。这违反「core 不认识 plugin」：插件改名或改字段，`runHealth` 会静默记 0，并写进 terminal entry。`check-layers` 只查 import，拦不住（#134）。

## Decision

core 在 `tools.ts` 定义 `ChildRun = { usage: Usage } & Record<string, unknown>`，约定工具用 `details.childRuns: ChildRun[]` 报告它跑过的子会话。`runHealth` 只读这个字段，不看工具名。两个 spawn 工具都写成 `{ childRuns: [...] }`，单个与批量形状对称，并用 `satisfies ChildRun[]` 绑定到 core 的类型。`check-layers` 规则 3 新增一项：从 plugins 的工具定义（`name: "x"` 和 `*_TOOL_NAME = "x"`）收集工具名，core 源码出现这些字符串字面量即失败；core 测试不受限。

## Alternatives considered

在 details 上同时保留旧的 `usage` / `results` 和新的 `childRuns`：同一份 usage 存两次，违反「一个事实一个定义」，G4 也不要求兼容。把 `runHealth` 整体移出 core，或把 manifest 里的插件报表类型（tier、shadow、scorer、entryScored）迁到 plugins：更彻底，但改动面约 1000 行，而且 #132 的对照结论可能让编译器 manifest 大改，现在搬多半返工。本次只做工具名这一处，其余留到 #132 之后再定。

## Consequences

spawn 工具的 details 形状变了：`details.sessionId` 等字段移到 `details.childRuns[0]`，批量的 `details.results` 改名为 `details.childRuns`。旧会话 Log 里的子会话不再计入 `health.subtasks`（预发布，不做迁移）。core 里 `RunHealth.subtasks` 字段名不变。manifest 结构仍由 core 定义，分层上的这部分欠账照旧。

## Testing

`packages/core/test/health.test.ts` 改用不叫 spawn 的工具名（`delegate`、`fan_out`）加 `childRuns` 驱动；修复前 `subtasks.count` 为 0，修复后为 3。另一个工具只带顶层 `details.usage`、没有 `childRuns`，不计入。`scripts/__tests__/check-layers.test.ts` 新增用例：core 源码里出现插件工具名字面量时报规则 3，注释和 core 测试不报。用 main 的源码跑新版 `check-layers`，报出 `health.ts` 的三处。plugins 的 subtask 测试改从 `soleChildRun`（`@sage/core/testing`）读子会话字段。
