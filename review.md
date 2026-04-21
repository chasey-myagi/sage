# Port Review — CHA-30: TUI 渲染系统

## 审查结果：⚠️ REVISE

| 维度 | 分数 | 关键发现 |
|------|------|---------|
| Logic Correctness | 3 | compute_message_height 对长行高度高估；sticky scroll 底部出现空白 |
| Plan Adherence | 5 | 5/5 实现点全部到位 |
| Optimization Correctness | 5 | render-time sticky、saturating_* 无副作用 |
| Idiomatic Quality | 4 | 整体符合 Rust 惯用；测试覆盖率良好 |
| OSS Readiness | 4 | 模块文档中出现 "pi-mono" 内部代号 |

---

### 必须修复

#### [L1] cargo fmt 失败 — 3 个文件共 12 处格式问题

`cargo fmt --check` 退出码非零，需在 push 前通过。

文件：
- `crates/ai/src/model_pricing.rs`（3 处）
- `crates/coding-agent/src/agent_session.rs`（3 处）
- `crates/coding-agent/src/modes/interactive/mod.rs`（6 处）

修复：在 worktree 根目录执行 `cargo fmt`，然后提交。

---

#### [L2] `compute_message_height` 对换行内容高度高估

**文件**: `crates/coding-agent/src/modes/interactive/mod.rs:334-350`

**问题**: 当前公式对所有行使用 `effective = inner_width - prefix_len`：

```rust
let effective = inner_width.saturating_sub(prefix_len).max(1);
let count: u16 = msg.content.lines().map(|line| {
    let n = line.chars().count() as u16;
    if n == 0 { 1 } else { n.div_ceil(effective) }
}).sum();
```

但 ratatui 的 Paragraph+Wrap 实际上是对整行字符做截断：第一 ratatui 行包含 `prefix + 内容前 (inner_width - prefix_len)` 个字符，但之后的续行每行有 **完整的 inner_width** 列可用，不再减 prefix。

**实例**（inner_width=20, prefix_len=5, content="a"×100）：
- 当前计算：`ceil(100/15) = 7` 行
- ratatui 实际渲染：`ceil(105/20) = 6` 行
- 测试 `compute_message_height_wrapping` 断言 `h == 7`，但 ratatui 渲染 6 行

**后果**：sticky 模式下 `scroll_top = total - viewport_height`，total 被高估，ratatui 被要求跳过更多行，底部出现空白行，最新消息不可见。

**修复方案**：

```rust
fn compute_message_height(msg: &ChatMessage, inner_width: u16) -> u16 {
    let prefix_len: u16 = match msg.role {
        MessageRole::User => 5,
        MessageRole::Assistant => 6,
        MessageRole::System => 8,
    };
    let first_capacity = inner_width.saturating_sub(prefix_len).max(1);
    let total: u16 = msg.content.lines().map(|line| {
        let n = line.chars().count() as u16;
        if n == 0 {
            1
        } else if n <= first_capacity {
            1
        } else {
            // first row uses first_capacity, remaining rows use inner_width
            1 + (n - first_capacity).div_ceil(inner_width.max(1))
        }
    }).sum();
    total.max(1)
}
```

同步更新 `compute_message_height_wrapping` 测试期望值：`assert_eq!(h, 6)`。

---

### 建议修复（非阻塞）

#### [S1] 'G' 键 modifier 兼容性

**文件**: `mod.rs:226-230`

```rust
(KeyCode::Char('G'), KeyModifiers::NONE)
    if self.input_buffer.is_empty() =>
{ self.is_sticky = true; }
```

在 Kitty keyboard protocol（kitty、Ghostty 新版本等）下，Shift+G 会上报 `KeyModifiers::SHIFT`，导致该分支永远不匹配，按键落入通用 `Char(c)` 分支被插入 buffer。

建议改为接受 `NONE | SHIFT`：

```rust
(KeyCode::Char('G'), m)
    if (m == KeyModifiers::NONE || m == KeyModifiers::SHIFT)
        && self.input_buffer.is_empty() =>
{ self.is_sticky = true; }
```

#### [S2] `init()` 与 `run()` 同时调用导致 User 消息双重插入

**文件**: `mod.rs:123-129` (`init`) 和 `mod.rs:143-155` (`run`)

两处都在 `initial_message` 非空时 push User 消息。若调用方先调 `init()` 再调 `run()`，消息列表为 `[User, User, Assistant]`，第一条用户消息出现两次。

建议：在 `run()` 中检查是否已有消息，或移除 `init()` 中的 push，改在 `run()` 中统一处理。

#### [S3] 模块文档 OSS 清理

**文件**: `mod.rs:3`

```rust
//! Translated from pi-mono `packages/coding-agent/src/modes/interactive/interactive-mode.ts`.
```

`pi-mono` 是内部代号，建议改为公开描述（例如引用 CC 的 `interactiveMode.ts`）或删除来源注释。
