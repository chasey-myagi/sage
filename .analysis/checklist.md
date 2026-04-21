# Checklist

## Files
- [ ] `utils/permissions/permissions.ts` — 核心权限检查引擎，处理 allow/deny/ask 决策、auto mode 分类器集成、规则匹配 — complexity: high
- [ ] `utils/permissions/permissionsLoader.ts` — 从磁盘加载权限规则，管理多源规则（userSettings/projectSettings/localSettings）持久化 — complexity: medium
- [ ] `utils/permissions/permissionRuleParser.ts` — 解析权限规则字符串，支持转义括号，处理工具名称兼容性 — complexity: medium
- [ ] `tools/EnterPlanModeTool/EnterPlanModeTool.ts` — 进入 Plan 模式，设置权限上下文，支持 interview phase — complexity: low
- [ ] `tools/ExitPlanModeTool/ExitPlanModeV2Tool.ts` — 退出 Plan 模式，支持同学计划批准流程，恢复权限上下文 — complexity: medium
