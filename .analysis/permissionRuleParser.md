# Analysis: utils/permissions/permissionRuleParser.ts

Summary: 权限规则字符串解析器，解析 "Tool" 或 "Tool(content)" 格式，支持括号和反斜杠转义，处理 legacy 工具名称兼容性映射。

## Issues（移植到 Rust 会出错的地方）

- [x] ISSUE [HIGH]: 字符串索引和字符操作（str[i], str.length）
  Impact: TypeScript 的字符串是 utf-16，索引不安全；Rust 的 &str 是 utf-8，需要迭代器而非索引
  Suggestion: 使用 chars().nth(i) 或 bytes 迭代，避免 O(n) 字符索引

- [x] ISSUE [HIGH]: 转义序列的正确顺序和复杂性
  Impact: 当前的手动 replace() 链容易出错（例如转义顺序不当导致 \\\\ 被误解）
  Suggestion: 使用解析库（pest、nom）构建严格的 PEG 或更清晰的状态机

- [x] ISSUE [MEDIUM]: 负数索引隐含和边界检查（findLastUnescapedChar 从末尾遍历）
  Impact: Rust 没有负数索引，遍历逻辑需要显式处理
  Suggestion: 使用 .rev() 迭代器链替代 for i in ... 倒序

- [x] ISSUE [MEDIUM]: 动态 Map 对象用作 legacy 名称映射（LEGACY_TOOL_NAME_ALIASES）
  Impact: Object 字面量在 Rust 中需要 match 或 HashMap，性能和编译时检查不同
  Suggestion: 使用 const LEGACY_ALIASES: &[(oldname, newname)] = &[...] 或 phf::Map<&'static str, &'static str> 以零成本

- [x] ISSUE [LOW]: 魔法字符串（"\\"、"("、")"）
  Impact: 容易拼写错误，Rust const 可以避免
  Suggestion: const ESCAPE_CHAR: char = '\\'; const PAREN_OPEN: char = '(';

## Optimizations（Rust 能做得更好的地方）

- [x] OPT [SAFETY]: 使用类型安全的规则表示替代字符串
  Why better: 当前返回的 PermissionRuleValue { toolName: String, ruleContent?: String } 容易包含非法的空字符串或括号
  Suggestion: enum RuleContent { None, Wildcard, Content(String) }，或 newtype RuleContent(String) with validation

- [x] OPT [SAFETY]: 使用字符级 parser（pest/nom）替代手写的字符扫描
  Why better: 手写状态机易出 off-by-one 错误（特别是转义逻辑），parser combinator 更清晰
  Approach: 定义 Pest grammar: rule = toolname ~ ("(" ~ content ~ ")")? ; content = (escaped_char | normal_char)*

- [x] OPT [PERF]: 避免 O(n²) 的反复规范化（roundtrip parse-serialize）
  Why better: addPermissionRulesToSettings 中的 existingRulesSet 每次都规范化，可以预先缓存
  Suggestion: 在加载时规范化一次，存储 (original: String, normalized: String) 对

- [x] OPT [IDIOM]: 返回 Result 而非 null fallback
  Why better: 当前 permissionRuleValueFromString 遇到错误时静默返回 toolName，无法区分成功和失败
  Approach: permissionRuleValueFromString(...) -> Result<PermissionRuleValue, ParseError>

- [x] OPT [IDIOM]: 使用 const 函数替代运行时工具名称映射
  Why better: legacy 名称是静态的，Rust const 可以在编译时求值
  Approach: const fn normalize_legacy_tool_name(name: &str) -> &'static str { match name { ... } }
