# 下一阶段开发规划（v0.7.5 → v0.9.0 收尾）

> 基于 2026-04-17 的代码实况核对（见 `docs/TODO.md` 勾选状态）。
>
> 当前状态：v0.6 构建流水线 ✅ / v0.7 身份模型 ◐ / v0.8 daemon ✅ / v0.9 Hook 3/12 事件 + Harness 基础。
> 目标：补齐知识系统闭环 → Hook 扩展 → Wiki 自维护 → Channel 接入 → Harness+Craft 飞轮。

## 优先级原则

1. **先打通闭环，再堆新特性**。当前 Agent 有嘴（LLM）、有手（Tools/VM）、有身体（workspace），但没有「沉淀」——因为 TaskRecord 采集缺位、wiki 自维护缺位。优先把这条反馈链闭合。
2. **Hook 扩展比 Craft 早**。Craft 评分系统依赖 PostToolUse token 计量（需 MetricsCollector）、CraftEvaluation 触发（需 PreCompact 或 Stop hook 的扩展行为）。所以 Hook / Metrics 先行。
3. **Channel 晚于 Hook**。FeishuChannel 需要 SessionStart hook 注入 `channel_hints`，所以等 Sprint 6 Hook 扩展后再做。
4. **技术债异步清理**。v0.1.0 遗留（CancellationToken / Arc newtype / Registry 实例化）不阻塞任何功能，穿插在 sprint 之间当热身。

---

## Sprint 5 — 知识采集基础（v0.7.5）

**目标**：把 Agent 每次执行的「指标」和「经验包」持久化，为后续 Wiki / Craft / 评分提供数据源。

### 任务

#### S5.1 TaskRecord 采集（MetricsCollector）

补齐字段：
```rust
pub struct TaskRecord {
    // 已有
    pub task_id: String,      // ULID
    pub agent_name: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub turn_count: u32,
    pub tool_call_count: u32,
    pub compaction_count: u32,
    pub success: bool,
    // 新增
    pub config_hash: String,           // sha256(config.yaml)
    pub started_at: u64,               // unix ms
    pub ended_at: u64,
    pub duration_ms: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub tool_error_count: u32,
    pub failure_reason: Option<String>,
    pub session_type: SessionType,     // 过滤用
    pub crafts_active: Vec<String>,    // Sprint 9 用，暂填空 Vec
}
```

实现 `MetricsCollector`（放在 `sage-runner/src/metrics.rs`）：
- 持有 `TaskRecord` + `Arc<Mutex<...>>` + workspace_host
- `subscribe(event_rx)`：后台 task，消费 AgentEvent 流累积字段
- `finalize(success, failure_reason)`：序列化写入 `<workspace>/metrics/<task_id>.json`
- `update_summary()`：维护 `<workspace>/metrics/summary.json`（最近 50 条滚动）

集成点：`SageEngine::run()` 和 `SageSession::send()` 创建 Collector 并订阅事件；仅 `SessionType::UserDriven` 触发 finalize 写盘。

#### S5.2 Skill frontmatter 解析 + 懒加载

当前 `load_skill_files` 把整个 `.md` 塞进 system prompt，既浪费 token 又没法排序。改为：

```rust
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub when_to_use: Option<String>,
    pub allowed_tools: Vec<String>,
    pub agent: Option<String>,        // 过滤当前 agent
    pub hooks: Option<Vec<String>>,   // Sprint 6 激活 hook 用，先解析占位
    pub score: f32,                   // 初始 1.0，Sprint 9 更新
    pub version: u32,
    pub body_path: PathBuf,           // 懒加载入口
}
```

流程：
- 启动扫描时只解析 frontmatter（用 `yaml-front-matter` crate 或自写 `---` 分隔）
- system prompt 注入时按 `(score desc, name asc)` 排序，仅塞 `name + description + when_to_use`
- TUI `/skill-name` 触发时才 `fs::read_to_string(body_path)`，做 `$ARGUMENTS` 替换再注入对话

frontmatter 缺失或格式错误时退化到当前的整体注入（向后兼容）。

#### S5.3 init_agent 扩展 — wiki 骨架

当前 `sage init` 只建 `AGENT.md / memory/MEMORY.md / config.yaml / workspace/`。扩展生成：

```
workspace/
├── SCHEMA.md                    # wiki 锚点，从内置模板写入
├── raw/sessions/.gitkeep
├── wiki/
│   ├── index.md                 # 空目录表
│   ├── log.md                   # processed-session ledger 起始行
│   ├── overview.md
│   └── pages/.gitkeep
├── metrics/.gitkeep
├── craft/.gitkeep
└── skills/.gitkeep
```

`SCHEMA.md` 模板嵌入到 binary，对齐 `sage-wiki/AGENTS.md` 约定。

### 验收

```bash
# 新 agent 初始化后目录结构齐全
sage init --agent test && tree ~/.sage/agents/test/workspace

# UserDriven 任务跑完后有指标文件
sage run --config ... --message "..."
ls ~/.sage/agents/test/workspace/metrics/*.json

# Skill frontmatter 生效：system prompt 只注入概要，body 按需加载
SAGE_LOG=debug sage chat --agent test
```

### 工作量估算：~1.5 周，单人

---

## Sprint 6 — Hook 体系统一（v0.9.0 H1 扩展）

**目标**：把 3 个独立 hook trait 重构为统一 `HookEvent` 枚举，并扩展 4 个关键事件（SessionStart / SessionEnd / PreCompact / PostCompact）。顺手做 API 重命名与 stdin JSON contract。

### 任务

#### S6.1 HookEvent 枚举 + broadcast 广播

```rust
// sage-runtime/src/hook.rs（新文件）
pub enum HookEvent {
    SessionStart { session_id, agent_name, model },
    UserPromptSubmit { text },
    PreToolUse { tool_name, tool_call_id, args },
    PostToolUse { tool_name, tool_call_id, args, is_error, duration_ms },
    PreCompact { tokens_before, message_count },
    PostCompact { tokens_before, tokens_after, messages_compacted },
    Stop { stop_reason, turn_count, last_assistant_message },
    SessionEnd { duration_ms, turn_count, success },
}

pub enum HookOutcome { Allow, Intervene { message: String } }
pub trait HookHandler: Send + Sync {
    async fn handle(&self, event: &HookEvent) -> HookOutcome;
}
```

运行时用 `tokio::sync::broadcast::channel(256)` 广播 HookEvent。agent_loop 在对应 lifecycle 点 emit，handler 订阅 receiver 并决定是否 Intervene。

保留现有 `BeforeToolCallHook` / `AfterToolCallHook` / `StopHook` trait 作为 v0.1 兼容层（由统一层适配），下个 minor 删除。

#### S6.2 扩展 4 个关键事件集成点

| 事件 | 触发位置 | exit 2 语义 |
|-----|---------|------------|
| `SessionStart` | `SageSession::new` 建完 engine 就绪后 | 中断启动，stderr 作为初始化失败原因 |
| `SessionEnd` | `SageSession::drop` 或显式 close | 仅观测，不可阻断 |
| `PreCompact` | `agent_loop.rs` 触发 compact 前 | 阻断 compact（给 Agent 机会先写 MEMORY） |
| `PostCompact` | compaction.rs 摘要完成后 | 仅观测 |

#### S6.3 Stop hook stdin JSON contract

替换当前的 `SAGE_*` 环境变量，改为 stdin 写入 JSON：
```json
{"event":"Stop","session_id":"...","agent_name":"...","model":"...","turn_count":5,"stop_reason":"EndTurn","last_assistant_message":"...","task_id":"..."}
```
`execute_hook` 增加 `stdin_json: Option<Value>` 参数，fork 后 `child.stdin.write_all(...)`。向后兼容：环境变量保留，但文档标注 deprecated。

#### S6.4 API 重命名 + lifecycle hooks

- `on_before_tool_call` → `on_pre_tool_use`（engine.rs Builder + 3 处 wrapper）
- `on_after_tool_call` → `on_post_tool_use`
- 新增 `on_session_start` / `on_session_end` / `on_pre_compact` / `on_post_compact` builder 方法
- 保留旧名 6 个月 deprecation 周期（`#[deprecated]`）

#### S6.5 config.yaml hooks 扩展

`HooksConfig` 增加字段：
```yaml
hooks:
  session_start:
    - command: "..."
  session_end: [...]
  pre_compact: [...]
  post_compact: [...]
```

### 验收

- 853+ tests 继续通过
- 新写 8+ tests 覆盖 SessionStart/End/PreCompact/PostCompact 的 exit 0/exit 2 路径
- `/hooks.md` 设计文档更新（现在是 3 事件，改为 8 事件表）

### 工作量估算：~2 周

---

## Sprint 7 — Wiki 自维护闭环（v0.8.0 收尾）

**目标**：让 Agent 在 IDLE 时段自动归档会话、蒸馏知识到 wiki，形成「用 → 积累 → 回顾 → 用得更好」的飞轮。

### 任务

#### S7.1 sage-wiki skills 安装链路

`sage-wiki/skills/` 有 5 个 skill（wiki-init / wiki-ingest / wiki-query / wiki-lint / wiki-update）。方案：
- `sage init --agent <name>` 时把这 5 个 skill 的 `SKILL.md` 拷贝到 `~/.sage/skills/`（平台级）
- 或通过 `sage-cli/build.rs` 把 sage-wiki 子模块的 SKILL.md 嵌入二进制，`sage install-skills` 子命令写出
- 首次运行时若缺 skill 自动补

选方案 B（`include_str!` 嵌入），避免引入子模块依赖。

#### S7.2 会话归档到 raw/sessions/

在 `SageSession::drop` 或 `SessionEnd` hook 中，将完整对话序列化为 JSONL 追加到 `<workspace>/raw/sessions/<session_id>.jsonl`。格式对齐 `sage-wiki/AGENTS.md` 的约定（message 数组 + metadata）。

只有 `SessionType::UserDriven` 归档；HarnessRun / WikiMaintenance / CraftEvaluation 不归档（避免自指污染）。

#### S7.3 DaemonLoop IDLE 触发 WikiMaintenance

daemon 增加内部状态：
```rust
enum DaemonState { Idle, Processing, WikiMaintenance }
```

在 `handle_client` 循环空闲（accept 超时或 client 断开）时：
1. 读 `wiki/log.md` 的 processed sessions 集合
2. 扫描 `raw/sessions/` 统计未处理数量
3. 若 ≥ `wiki.trigger_sessions`（默认 3）且距离上次维护 ≥ `wiki.cooldown_secs`（默认 1800s）：
   - 切换 state 到 WikiMaintenance
   - 启动新 session（SessionType::WikiMaintenance）
   - 注入 system prompt：「你现在是 wiki 维护者，使用 wiki-ingest skill 处理未归档 session」
   - 运行到 `StopAction::Pass`

维护期间若有 client 连入，消息入队列，维护完成后优先处理。

#### S7.4 WikiConfig

`AgentConfig` 增加：
```rust
pub struct WikiConfig {
    #[serde(default = "default_trigger_sessions")]
    pub trigger_sessions: u32,   // 默认 3
    #[serde(default = "default_cooldown_secs")]
    pub cooldown_secs: u64,      // 默认 1800
    #[serde(default)]
    pub enabled: bool,           // 默认 false，显式 opt-in
}
```

### 验收

```bash
# 跑 3 个 UserDriven 任务后，daemon idle 会触发 wiki 维护
sage start --agent feishu
# ... 发 3 条消息 ...
sleep 60
ls ~/.sage/agents/feishu/workspace/wiki/pages/  # 看到生成的页面
cat ~/.sage/agents/feishu/workspace/wiki/log.md  # 看到 processed 条目
```

### 工作量估算：~2.5 周（wiki prompt 调优 + skill 嵌入打磨）

### 风险

- WikiMaintenance session 消耗额外 LLM token，需把 `wiki.enabled` 默认设为 false
- 维护期间用户消息的排队逻辑不做好会导致感知卡顿 → 必须有 daemon 级抢占机制（收到 client 连入立刻打断维护）

---

## Sprint 8 — Channel 真实接入（v0.9.x C3/C4）

**目标**：把 `ChannelAdapter` 从 stub 扩展为真实 trait，完成 FeishuChannel 端到端回路（飞书群 → daemon → 回复）。

### 任务

#### S8.1 ChannelAdapter trait 扩展

```rust
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn channel_hints(&self) -> &str;                       // 平台格式说明
    fn visibility_filter(&self) -> Visibility;              // 默认 Visibility::User
    async fn send(&self, event: AgentEvent) -> Result<()>;
}
```

#### S8.2 FeishuChannel 完整实现

- `crates/sage-channel-feishu/`（新 crate，隔离 axum + reqwest 依赖）
- axum webhook handler：`POST /webhook/feishu` 接收 `im.message.receive_v1`
- 签名校验（`X-Lark-Signature` HMAC）
- 路由到 Unix socket：根据 `chat_id` 映射到 `sage-<agent>.sock`，写 `ClientMsg::Send`
- 消息去重：LRU cache 最近 1000 个 `message_id`，防 webhook 重试
- `send`：按 card schema POST `/open-apis/im/v1/messages`，access_token 自动刷新

#### S8.3 config.yaml channel 字段

```yaml
channel:
  type: feishu
  app_id: "cli_xxxx"
  app_secret: "${FEISHU_APP_SECRET}"    # env 展开
  verification_token: "..."
  webhook_port: 3400
```

`sage start --agent feishu` 读到 channel 字段时同时起 webhook server（subprocess 或 async task）。

#### S8.4 SessionStart hook 注入 channel_hints

利用 Sprint 6 的 SessionStart hook 在 system prompt 追加 `channel_hints()`，让 Agent 知道当前输出目标是飞书卡片而非 TUI。

### 验收

飞书群 @sage → daemon 收到 → Agent 回复 → 飞书群出卡片消息，全链路跑通。

### 工作量估算：~2 周

---

## Sprint 9 — Harness 声明式 criteria + 并发（v0.9.0 H2 收尾）

**目标**：把 `sage test` 从「只支持 eval 脚本出口码」升级为「声明式多维断言 + 并发 + junit 报告」，用于 CI 守门。

### 任务

#### S9.1 声明式 criteria

```yaml
cases:
  - name: "查询今日日历"
    message: "帮我查今天有哪些会议"
    criteria:
      - { check: output_contains, pattern: "会议|日历|日程" }
      - { check: tool_called, tool: bash }
      - { check: token_budget, max_input_tokens: 2000, max_output_tokens: 500 }
      - { check: turn_budget, max_turns: 5 }
      - { check: no_error }
```

实现在 `harness.rs`：每类 check 一个函数，顺序执行，全部 pass 才判 Pass。`eval` 脚本保留为「自定义 escape hatch」。

#### S9.2 --case / --parallel / --reporter

- `--case "查询今日日历"` 支持按名字过滤（exact match 或 glob）
- `--parallel N` 用 `futures::stream::iter().buffer_unordered(N)`
- `--reporter junit` 输出 JUnit XML（兼容 GitHub Actions `mikepenz/action-junit-report`）
- `--output results/` 把 JSON/XML 落到目录

#### S9.3 HarnessRun 运行时隔离

`SessionType::HarnessRun` 跑时：
- MetricsCollector 不写 TaskRecord（但仍采集，塞到 TestOutcome 的 token 统计）
- daemon 不触发 Wiki 维护
- 不归档到 raw/sessions/

### 验收

```bash
sage test --suite feishu-regression.yaml --parallel 4 --reporter junit --output ci/
```
GitHub Actions 能直接可视化结果。

### 工作量估算：~1.5 周

---

## Sprint 10 — Craft 体系（v0.9.0 S1）

**目标**：让 Agent 能把「反复用到的 SOP / 脚本 / 模板」沉淀成 Craft，自动评分，低效的自动触发 CraftEvaluation 重写。

### 任务

#### S10.1 craft_manage 工具

注册到 `ToolRegistry`：
- `create(name, type, content, tags, allowed_tools)` — 写 `<workspace>/craft/<name>/CRAFT.md`
- `edit(name, content)` — version++
- `delete(name)` — 软删除（移到 `craft/.trash/`）
- `list()` — 返回所有 craft + score
- 全局 `~/.sage/skills/` 只读，工具拒绝修改

#### S10.2 Craft 与 Skill 统一扫描 + 懒加载

复用 Sprint 5 的 SkillMeta 扫描器，把 `workspace/craft/` 也纳入。注入 system prompt 时合并排序：全局 skills 优先 + workspace crafts 次之。

#### S10.3 效率评分采集

扩展 MetricsCollector 在 TaskRecord 中写 `crafts_active: Vec<String>`（本 session 被 `/slash` 调用的 craft 列表）。

离线脚本 / 定时 task：扫 `metrics/*.json`，按 craft 聚合 avg_tokens，更新每个 craft frontmatter：
```yaml
---
score: 0.73           # best_tokens / avg_tokens
tokens_avg: 1840
tokens_best: 1342
usage_count: 8
last_scored: 2026-05-14
---
```

#### S10.4 CraftEvaluation session

score < 0.5 且 usage_count ≥ 5 → 触发 `SessionType::CraftEvaluation`，system prompt 注入「请重写这个 craft，提高 token 效率」+ craft 原文。

### 验收

跑 5 次 `/feishu-schedule` 后看 craft 文件有 score 更新，手动劣化一个 craft 看是否触发 evaluation。

### 工作量估算：~2 周

---

## Sprint 11 — v0.1.0 技术债清理

**目标**：清理架构遗留，为 1.0 发布做准备。可以拆成小 PR 穿插在其他 sprint 中。

- [ ] `CancellationToken` 贯通 agent_loop → tool backend → LLM stream；实现 `engine.cancel()` + `Ctrl+C` 优雅打断
- [ ] 6 个 `Arc*` newtype wrapper（ArcProvider / ArcBeforeHook / ArcAfterHook / ArcTransformContextHook / ArcStopHook / ArcTool）改用 blanket impl 或 `Arc<dyn ...>` 直接作为 `Box<dyn ...>` 的轻量适配
- [ ] `llm/registry.rs` 全局 `static REGISTRY` → `SageEngine` 实例持有 `ApiProviderRegistry`
- [ ] `sage-cli/src/chat.rs` Ctrl+C 优雅退出（signal handler + session flush）

### 工作量估算：~1 周（分散进 Sprint 5-10）

---

## 整体时间线

```
Sprint 5   知识采集基础     1.5w  ▓▓▓
Sprint 6   Hook 体系统一    2.0w    ▓▓▓▓
Sprint 7   Wiki 自维护      2.5w        ▓▓▓▓▓
Sprint 8   Channel 接入     2.0w              ▓▓▓▓
Sprint 9   Harness 增强     1.5w                  ▓▓▓
Sprint 10  Craft 体系       2.0w                     ▓▓▓▓
Sprint 11  技术债清理       1.0w  (分散穿插)
------------------------------------------
合计约 12 周（单人，含 buffer）
```

并行度提示：Sprint 6 H1 Hook 扩展可以和 Sprint 5 的 S5.1 MetricsCollector 并行（MetricsCollector 可以先订阅现有 AgentEvent，Sprint 6 完成后再切到 HookEvent broadcast）。Sprint 9 与 Sprint 10 也可以并行（Harness criteria 主要动 CLI 层，Craft 主要动 runner / 工具层）。

## 不做什么

- **v0.5.0 高级沙箱**（OCI / VM pool / snapshot）— 个人 Agent 场景不需要，推迟到 v1.x 多租户
- **Slack / WebUI Channel** — 先把 Feishu 做透，模式稳定后再复制
- **OpenTelemetry** — JSON 日志够用，OTel 等真要接生产再说
- **rustyline REPL 升级** — 当前 tokio stdin 够用，Ctrl+C 处理在 Sprint 11 一并做
- **v1.0 crates.io 发布** — 在 Sprint 10 完成后再评估 API 稳定性

## 关键风险

1. **Sprint 6 Hook 重构破坏既有测试**：853+ tests 里大量用 `on_before_tool_call`，需兼容层 + deprecation 过渡至少一个 minor
2. **Sprint 7 WikiMaintenance 成本**：每次维护额外 LLM 开销，必须默认 `enabled: false`，并提供「干跑」模式看预期 token 消耗再决定
3. **Sprint 10 Craft 评分数据不足**：冷启动阶段没足够样本，score 会剧烈震荡；首版用保守阈值（usage_count ≥ 5 才评分）避免噪音
