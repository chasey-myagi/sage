# Analysis: utils/permissions/permissionsLoader.ts

Summary: 权限规则加载器，从磁盘读取多个设置源（policySettings/userSettings/projectSettings/localSettings）的 JSON 权限规则，支持版本兼容性（legacy 工具名称规范化），并提供规则增删改查操作。

## Issues（移植到 Rust 会出错的地方）

- [x] ISSUE [HIGH]: 可选的 null 返回和链式检查（settingsData?.permissions）
  Impact: TypeScript 的可选链接 (?.`) 隐藏了 null 传播，Rust 强制显式 Option 处理
  Suggestion: 使用 Option::map_or, Option::and_then，模式匹配替代可选链接

- [x] ISSUE [HIGH]: 异步文件 I/O 但同步调用（readFileSync, writeFile）
  Impact: Rust 需要选择 sync（std::fs）或 async（tokio::fs），混用会导致死锁
  Suggestion: 如果在异步上下文中，使用 tokio::fs；如果同步，使用 std::fs

- [x] ISSUE [HIGH]: JSON 序列化容错（safeParseJSON with validation disabled）
  Impact: SettingsJson 类型可能包含无效数据，Rust 的 serde 会拒绝
  Suggestion: 定义严格的 PermissionSettingsJson 结构体，或使用 serde_json::Value 并手动验证

- [x] ISSUE [MEDIUM]: Set 操作去重（new Set, has, add）
  Impact: Rust 的 HashSet 要求泛型参数实现 Eq + Hash，String 可以但需要 owned
  Suggestion: 使用 HashSet<String>，注意生命周期

- [x] ISSUE [MEDIUM]: 嵌套的对象更新（...settingsData, permissions: {...existing, [rule.ruleBehavior]: [...]}）
  Impact: Rust 没有内置的深拷贝+更新语法，需要手动构造
  Suggestion: 使用 struct 字段赋值或 builder，考虑 structstruct 库简化深层更新

- [x] ISSUE [MEDIUM]: 错误处理的 catch-all 返回（catch { return null }）
  Impact: 这会隐藏 I/O 或 JSON 解析错误，Rust 需要显式错误传播或记录
  Suggestion: 所有 I/O 操作返回 Result，调用端决定是否 unwrap/suppress

- [x] ISSUE [LOW]: 文件路径作为字符串操作（getSettingsFilePathForSource）
  Impact: TypeScript 字符串可以包含无效路径，Rust 应使用 PathBuf
  Suggestion: 返回 Option<PathBuf>，使用 path.exists() 等类型安全的方法

## Optimizations（Rust 能做得更好的地方）

- [x] OPT [SAFETY]: 使用 serde 和类型验证替代手动 JSON 处理
  Why better: serde 编译时检查 JSON 结构，防止字段拼写错误和类型不匹配
  Approach: #[derive(Serialize, Deserialize)] PermissionSettingsJson { permissions: PermissionMap }，serde_json::from_str()

- [x] OPT [SAFETY]: 路径安全性检查（Path normalization）
  Why better: Rust PathBuf 自动处理 .. 和符号链接，防止目录遍历
  Approach: 使用 std::fs::canonicalize() 或 path.normalize_components()

- [x] OPT [PERF]: 避免重复规范化（normalizeEntry roundtrip）
  Why better: 当前 permissionRuleValueToString(permissionRuleValueFromString(raw)) 被调用多次
  Suggestion: 缓存规范化结果或在加载时一次性规范化

- [x] OPT [IDIOM]: 返回类型使用 Result<T> 替代 boolean 或 null
  Why better: 当前返回 boolean/null 不能区分"失败"和"不存在"
  Approach: 使用 Result<(), LoadError> 或 Option<PermissionRule>

- [x] OPT [ERGONOMICS]: 链式 API 用于规则操作
  Why better: 当前的 add/delete/get 分离，Rust 可以提供 fluent builder
  Approach: PermissionRulesBuilder::new().add_allow(...).add_deny(...).save()
