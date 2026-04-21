# Analysis: tools/EnterPlanModeTool/EnterPlanModeTool.ts

Summary: 进入 Plan 模式的工具，触发权限上下文转换到 'plan' 模式，禁用条件编译特性，返回设计阶段的说明消息。

## Issues（移植到 Rust 会出错的地方）

- [x] ISSUE [HIGH]: 特征开关控制的可用性（feature('KAIROS') || feature('KAIROS_CHANNELS')）
  Impact: 需要在 Rust 中用 cfg! 或运行时检查表达相同的条件
  Suggestion: 使用 #[cfg(feature = "kairos")] 或 is_feature_enabled("kairos") 函数

- [x] ISSUE [MEDIUM]: setAppState 的副作用和闭包
  Impact: 当前使用闭包更新 appState，Rust 的可变引用受 borrow checker 约束
  Suggestion: 使用 &mut 引用或 interior mutability 模式（Cell/RefCell）

- [x] ISSUE [MEDIUM]: applyPermissionUpdate 的链式调用和结果可能为 null
  Impact: TypeScript 允许链式调用返回 null，Rust 需要显式处理 Option
  Suggestion: 返回 Result<PermissionContext, UpdateError>，使用 ? 操作符

- [x] ISSUE [LOW]: 静态字符串常数（searchHint, userFacingName）
  Impact: 与其他文件的字符串常数冗余，没有硬编码
  Suggestion: 提取到单独的 constants 模块或宏

## Optimizations（Rust 能做得更好的地方）

- [x] OPT [SAFETY]: Tool 特征使用 const fn 做标志查询
  Why better: isEnabled()、isConcurrencySafe() 等运行时调用可以编译时确定
  Approach: 为 Tool 特征添加 const fn is_enabled() 关联常量

- [x] OPT [IDIOM]: 返回值使用 struct Output 而非嵌套 map
  Why better: { data: { message: string } } 嵌套深，直接返回 Output struct 更清晰
  Approach: struct EnterPlanModeOutput { message: String }

- [x] OPT [ERGONOMICS]: 说明消息使用模板或资源文件
  Why better: 当前硬编码在 mapToolResultToToolResultBlockParam，难以本地化或更新
  Suggestion: 使用 lazy_static 加载 instructions.txt 或使用 include_str!() 宏
