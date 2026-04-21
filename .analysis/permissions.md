# Analysis: utils/permissions/permissions.ts

Summary: 核心权限检查引擎，实现了复杂的多层权限决策流程（规则→auto mode 分类器→模式转换→hooks），支持 deny/ask/allow 三态决策和多种权限模式（default/auto/dontAsk/plan/bypassPermissions）。

## Issues（移植到 Rust 会出错的地方）

- [x] ISSUE [BLOCKER]: Promise/async 异步模型
  Impact: Rust 需要显式选择运行时（tokio vs async-std vs 同步）
  Suggestion: 使用 tokio::spawn 和 async/await，wrapper 需要符合 Send + Sync 约束

- [x] ISSUE [HIGH]: 隐式 null/undefined 传播
  Impact: TypeScript 允许 undefined 在任何地方隐式出现，Rust 的 Option<T> 强制显式处理
  Suggestion: 每个可能为 null 的值都改为 Option<T>，使用 ? 操作符传播

- [x] ISSUE [HIGH]: 特征开关（feature flags）通过动态 require
  Impact: 条件编译时 Rust 需要静态 cfg!() 或运行时检查
  Suggestion: 使用 feature = "transcript_classifier" 等 cargo features 或运行时枚举

- [x] ISSUE [HIGH]: 复杂的对象浅拷贝（...prev, ...state）
  Impact: Rust 中深拷贝和浅拷贝有不同性能含义，对象更新需要显式 Clone
  Suggestion: 为状态对象实现 Clone，使用 struct 更新语法或 builder 模式

- [x] ISSUE [MEDIUM]: 动态导入的模块可能为 null（classifierDecisionModule, autoModeStateModule）
  Impact: Rust 不允许可能为 null 的模块引用，需要编译时或启动时验证
  Suggestion: 使用 lazy_static 或 once_cell，在初始化时确保模块存在，否则 panic 或返回错误

- [x] ISSUE [MEDIUM]: Map<string, PermissionRule> 和动态键操作
  Impact: HashMap 键类型必须是 Eq + Hash，字符串可以，但需要 owned String 或 &str 生命周期约束
  Suggestion: 使用 HashMap<String, PermissionRule>，getUpdatedInputOrFallback 需要生命周期标注

- [x] ISSUE [MEDIUM]: 嵌套的 Promise 错误处理（try/catch 和 error 链）
  Impact: Rust 的 ? 不会自动链接错误，需要使用 .map_err() 或自定义错误类型
  Suggestion: 定义 PermissionError 枚举，实现 From<T> 以自动转换，使用 ?

- [x] ISSUE [MEDIUM]: 回调和高阶函数（setAppState, getAppState）
  Impact: Rust 中闭包的生命周期和所有权约束严格，尤其涉及可变引用
  Suggestion: 使用 trait objects (Box<dyn Fn...>) 或 associated types，文档中标注 Send 要求

- [x] ISSUE [LOW]: 字符串内插和条件消息生成
  Impact: TypeScript 的模板字符串灵活，Rust 的 format! 受限
  Suggestion: 使用 format!() 宏，复杂模板考虑 askama 或 template 库

## Optimizations（Rust 能做得更好的地方）

- [x] OPT [SAFETY]: 显式错误类型替代隐式异常
  Why better: Rust 强制在编译时处理所有错误路径，消除 unhandled Promise rejections
  Approach: 定义 #[derive(thiserror::Error)] PermissionError 枚举，所有函数返回 Result<T, PermissionError>

- [x] OPT [SAFETY]: 状态一致性使用 Rust 的类型系统
  Why better: TypeScript 允许矛盾的状态（e.g. mode='plan' but isInPlanMode()=false），Rust 类型可以编译时禁止
  Suggestion: 用 newtype 或 enum 表达状态机不变量，e.g. enum PermissionMode { Plan(PlanState), Auto(...) }

- [x] OPT [PERF]: 避免不必要的深拷贝（Object.assign）
  Why better: Rust 的借用系统可以避免复制，只在必要时 clone
  Approach: 使用 &mut 传递可变引用而不是拷贝+赋值，让编译器强制正确的所有权

- [x] OPT [IDIOM]: 使用 enum 代替 string-based mode 判断
  Why better: TypeScript 的 'ask' | 'allow' | 'deny' 容易拼写错误，Rust enum 编译时检查
  Approach: enum PermissionBehavior { Allow, Deny, Ask } 配合 match 覆盖检查

- [x] OPT [IDIOM]: 规则匹配逻辑可用迭代器链重构
  Why better: 多个 flatMap + filter 循环可以用 iterator chains 表达，零成本抽象
  Approach: getAllowRules 用 .filter_map() / .collect() 替代 for + push

- [x] OPT [ERGONOMICS]: 权限决策返回值使用 Builder 模式
  Why better: 当前的 { behavior, message, updatedInput, decisionReason } 对象嵌套深，容易漏字段
  Approach: PermissionDecisionBuilder::new().deny().reason(...).build()
