# 审查结果：⚠️ REVISE

**审查范围**：CHA-50 Fix 提交 `bd05f7a`（resolver/chord/validate 移植）

| 维度 | 分数 | 关键发现 |
|------|------|---------|
| Logic Correctness | 4 | BUG-1/2/3 全部解决；chord 状态机语义正确；`Unbound` 在 chord 路径下不可达（见 L2） |
| Plan Adherence | 4 | resolver + chord + validate_user_config/check_duplicates 均已实现；OnceLock 优化大部分完成（6 个中漏 1 个） |
| Optimization Correctness | 4 | OnceLock 方案正确；4 个函数已修复，`decode_kitty_printable` 遗漏 |
| Idiomatic Quality | 4 | 无 unwrap（生产路径）、正确 Option/enum、OnceLock 使用规范 |
| OSS Readiness | 2 | `cargo fmt --check` 失败，keybindings.rs + keys.rs 均有格式差异 |

---

## 必须修复

### [L1] `cargo fmt --check` 失败 — 两处格式差异

**影响**：`cargo fmt --check` 返回非零，阻断 CI。

**文件 1**：`crates/coding-agent/src/core/keybindings.rs:95-96`

当前写法（跨行）：
```rust
static DEFAULT_APP_KEYS: OnceLock<HashMap<&'static str, &'static [&'static str]>> =
    OnceLock::new();
```
rustfmt 期望（单行）：
```rust
static DEFAULT_APP_KEYS: OnceLock<HashMap<&'static str, &'static [&'static str]>> = OnceLock::new();
```

**文件 2**：`crates/coding-agent/src/core/keybindings.rs:139-141`

当前写法：
```rust
return binding.as_keys().iter().any(|k| {
    !k.contains(' ') && tui::keys::matches_key(data, k)
});
```
rustfmt 期望：
```rust
return binding
    .as_keys()
    .iter()
    .any(|k| !k.contains(' ') && tui::keys::matches_key(data, k));
```

**文件 3**：`crates/tui/src/keys.rs` — `use std::sync::OnceLock` 位置问题（rustfmt 希望合并 `use std::sync::{...}` 块）

**修复方法**：在 worktree 执行 `cargo fmt`，然后 commit。

---

### [L2] `crates/tui/src/keys.rs:1231-1233` — `decode_kitty_printable()` 的 regex 未用 OnceLock

P1 修复了 `parse_kitty_sequence()`（`RE_KITTY_CSI_U`）中的相同正则，但 `decode_kitty_printable()` 是独立函数，仍在每次调用时重新编译：

```rust
pub fn decode_kitty_printable(data: &str) -> Option<char> {
    let re =
        regex::Regex::new(r"^\x1b\[(\d+)(?::(\d*))?(?::(\d+))?(?:;(\d+))?(?::(\d+))?u$").unwrap();
```

`decode_kitty_printable()` 是在 Kitty 模式下每个按键都会调用的热路径函数（与 `parse_kitty_sequence` 同频），正则重复编译会产生不必要分配。

**期望修复**：将 `decode_kitty_printable()` 改为复用 `RE_KITTY_CSI_U`：
```rust
pub fn decode_kitty_printable(data: &str) -> Option<char> {
    let re = RE_KITTY_CSI_U.get_or_init(|| {
        regex::Regex::new(r"^\x1b\[(\d+)(?::(\d*))?(?::(\d+))?(?:;(\d+))?(?::(\d+))?u$").unwrap()
    });
```

---

## 建议改进（非阻塞）

### [P1] `resolver.rs:ResolveResult::Unbound` 在 chord 路径下不可达

`resolve_key_with_chord_state` 中 `exact_action: Option<String>` 类型限制了它只能存字符串，当前数据模型（`KeybindingsConfig = HashMap<String, Vec<String>>`）无法表达 null-action（unbind）。`Unbound` 枚举变体存在但在 chord 路径下结构上不可达。

CC 源码 `resolver.ts` 的对应代码：
```typescript
if (exactMatch.action === null) {
  return { type: 'unbound' }
}
```

若后续支持用户通过 `null` 显式解除 chord 绑定，需要在数据模型层添加 `Option<String>` 表达 null-action，并在 `resolve_key_with_chord_state` 中处理此分支。当前行为是：解绑的 chord 键会返回 `None` 而非 `Unbound`，调用方无法区分"无绑定"与"显式解绑"。

### [P2] `resolver.rs` 缺少 `chordWinners` null-override-shadowing 逻辑

CC `resolver.ts` 通过 `chordWinners` Map 确保：如果一个 chord 绑定被 null-unbind 覆盖（`"ctrl+x ctrl+k": null`），前缀 `ctrl+x` 不应进入 chord-wait 状态、而应直接触发单键绑定。当前 Rust 实现的 `has_longer_chord` 布尔值不检查是否存在有效（非 null）的更长 chord，可能导致 chord-prefix 卡死。低优先级，因为 null-action 尚未建模。

### [P3] Context 过滤（`_contexts`）明确标为 deferred，文档充分

`resolve_key` 和 `resolve_key_with_chord_state` 均通过文档说明了 `_contexts` 保留供将来使用。无需在本轮修复，但引入带 context 元数据的绑定数据模型前，行为与 CC 存在差异。
