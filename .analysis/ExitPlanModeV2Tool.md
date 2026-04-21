# Analysis: tools/ExitPlanModeTool/ExitPlanModeV2Tool.ts

Summary: 退出 Plan 模式工具，处理计划批准流程（直接批准或通过 team lead mailbox），保存计划到磁盘，恢复权限上下文并处理 auto mode 恢复逻辑。

## Issues（移植到 Rust 会出错的地方）

- [x] ISSUE [BLOCKER]: 异步文件写入和错误处理（writeFile, persistFileSnapshotIfRemote）
  Impact: TypeScript Promise 式异步，Rust 需要 tokio 和显式 .await
  Suggestion: 使用 tokio::fs::write，包装在 async fn 中，使用 ? 处理错误

- [x] ISSUE [HIGH]: 复杂的条件路径和副作用链（isTeammate、isPlanModeRequired、feature flags）
  Impact: 多个条件判断和不同的返回路径，Rust 需要显式的状态机或 enum 来表达
  Suggestion: 定义 enum PlanExitContext { TeammateLeaderApproval, TeammateVoluntary, StandaloneUser }

- [x] ISSUE [HIGH]: 动态模块加载的 nullability（autoModeStateModule, permissionSetupModule）
  Impact: Rust 不允许可能为 null 的模块，需要 lazy_static/once_cell 保证初始化
  Suggestion: 使用 once_cell::sync::Lazy<AutoModeState> 或特征对象

- [x] ISSUE [HIGH]: 嵌套的 setAppState 和条件状态更新
  Impact: 当前有多个 context.setAppState 调用，每个都是副作用，难以追踪
  Suggestion: 提取到单独的 compute_exit_state() 函数，返回 NewState，最后一次更新

- [x] ISSUE [MEDIUM]: writeToMailbox 和 JSON 序列化（JSON approval requests）
  Impact: 需要与 teammate mailbox 的 IPC 机制，TypeScript 的 JSON 转换失败会导致 undefined
  Suggestion: 定义 struct ApprovalRequest，使用 serde_json::to_string()

- [x] ISSUE [MEDIUM]: 字符串插值和条件消息（notification text, instructions）
  Impact: format! 宏不支持三元运算符嵌套，复杂格式化需要重构
  Suggestion: 提取 format_exit_message() 函数，使用 if/else 构造字符串

- [x] ISSUE [MEDIUM]: 图灵级的条件恢复逻辑（restoreMode、strippedDangerousRules、autoWasUsedDuringPlan）
  Impact: 4 层嵌套的 if-else 和状态检查容易出 bug，Rust 类型系统无法表达
  Suggestion: 使用 state machine 或 builder 模式重构，编译时验证状态转换

- [x] ISSUE [LOW]: 常数类型转换（mode as AnalyticsMetadata_I_VERIFIED...）
  Impact: TypeScript 的类型铸造会被忽略，Rust 需要显式 From/Into impl
  Suggestion: 定义 impl From<PermissionMode> for AnalyticsMetadata

## Optimizations（Rust 能做得更好的地方）

- [x] OPT [SAFETY]: 状态转换的编译时验证
  Why better: 当前的多个 if/else 分支容易遗漏或重复，Rust enum 可以编译时穷举
  Approach: enum PermissionModeTransition { PlanToDefault, PlanToAuto, ... }，match 覆盖所有分支

- [x] OPT [PERF]: 避免多次 getAppState() 调用
  Why better: 当前有多个 context.getAppState() 调用，可能涉及锁竞争
  Suggestion: 在函数开头 let appState = context.getAppState()，传递引用

- [x] OPT [PERF]: 条件化加载 auto mode 恢复逻辑
  Why better: autoModeStateModule 和 permissionSetupModule 只在 TRANSCRIPT_CLASSIFIER feature 下使用
  Approach: 使用 #[cfg(feature = "transcript_classifier")] 条件编译，避免链接无用代码

- [x] OPT [IDIOM]: 使用 Result 而非 throw Error
  Why better: 当前 throw new Error(...) 中断执行，Rust 应返回 Result<Output, ExitError>
  Approach: ExitPlanModeResult { ... } 或 Result<PlanApprovalRequest, ExitError>

- [x] OPT [ERGONOMICS]: 分离关注点（approval logic / mode restoration / notification）
  Why better: 当前 call() 方法 200+ 行，混合多个职责
  Suggestion: 拆分为 compute_approval_request()、compute_exit_state()、build_notifications()

- [x] OPT [IDIOM]: Team communication 使用类型安全的信号
  Why better: 当前手动构造 { type: 'plan_approval_request', ... } JSON 对象容易遗漏或拼写错误
  Approach: enum TeamMessage { PlanApprovalRequest(ApprovalRequest), ... }，derive Serialize
