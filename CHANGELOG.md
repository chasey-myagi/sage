# Changelog

所有值得记录的变更按倒序列在这里。版本遵循 semver，但在 v0.1.0 之前
不保证 API / CLI / 磁盘布局稳定。

## [0.0.3] — 2026-04-18

### 发布阻断修复(pre-tag hotfix)

- **agent_loop 空参 tool call 无限循环 bug**。Kimi(以及任何
  OpenAI-completions 类 provider)对零参 tool call 会发 `arguments: ""`,
  而 `MessageAccumulator::finalize` 一边静默用 `{}` 写进 assistant
  message 历史、另一边 `prepare_tool_call` 走 "blocking call" 分支
  返回 `"Invalid tool call arguments: EOF"` —— 模型看到的 tool_result
  和它"自己发送的调用"(`{}`)对不上,进入循环:调 ls → blocked →
  "我再试一次" → blocked ... 20 turns max_turns 才兜住。
  新加 `coerce_tool_args(&str)` pure helper 统一语义:空/空白 → `{}`,
  非空合法 → 解析,非空非法 → `Err`。两个调用点走同一路径。regression
  test +4 锁死四种输入。
- **清零所有 release build warnings**(13 条): 删 chat.rs 死代码
  (`format_tool_start`/`format_tool_end`/`format_elapsed` 及其 10 条
  测试,被 `TerminalSink::emit` 内联替代);session_archive / wiki_trigger
  加模块级 `#![allow(dead_code)]` 并注释 v0.0.4 wire-up 目标;tui.rs
  protocol enum 加 `#[allow(dead_code)]`(schema 对称需要保留);
  engine.rs 删 unused `StopAction, StopContext` imports;config.rs 删
  unused `std::path::Path`;openai_responses.rs `get_service_tier_cost_multiplier`
  + harness.rs `TestCase.max_turns` 加 `#[allow(dead_code)]` 标
  pi-mono 对齐 / 前瞻字段。`cargo check --workspace` 现在 **零 warning**。

### 新功能

- **`sage skill evaluate --agent X --skill Y`** — 自演化手动触发。
  备份 `SKILL.md.bak.<ts>` → 构建 agent 自己的 engine → 发 system
  prompt 让 agent 用 `write` 工具重写 → 检查产物 → 失败回滚。
  sandbox 模式从 agent config 读,microVM 契约不被绕过。EvalSink
  把 `RunError` 和 tool error 转 stderr 可见;write tool 是否成功
  通过 AtomicBool 追踪,"model 没调 write" vs "model 重写后不变" vs
  "真的重写" 三路输出分流。**v0.0.4 将接 daemon 自动 tick 触发**
  (task #83)。
- **npm-style skill install** — `sage skill add --agent foo
  @scope/name` 或 `owner/repo`。走 `npx --yes skills add <spec>`
  shell-out。Windows 自动落 `npx.cmd` shim;`npx` 不在 PATH 时给
  Node.js 安装引导。
- **跨平台 release CI** — build.yml 从 3 matrix 扩到 5:macOS
  arm64 / macOS Intel x86_64 / Linux x86_64 musl / Windows x86_64
  msvc / sage-guest aarch64。全产物附 SHA256 sidecar。

### 修复 / 重构

- **craft_manage 并发 O_EXCL 回归测试**(task #81) — 10 tokio task
  同时 create 同名 skill,测试锁死"exactly one wins"契约。顺带发现
  并修复 `tokio::fs::File` 不显式 flush 会丢 write 的 data-loss bug
  —— `create_new(true) + write_all + flush` 三步走才是正解。
- **craft_manage 切 tokio::fs + serde_yaml**。手写 YAML frontmatter
  替换为 `serde_yaml::to_string(&CraftEntry)`;同一 `CraftEntry`
  结构双向用(写 + 读),减少 writer/parser 漂移。`validate_name`
  扩到 16 个 YAML-reserved 字符(原 4 个 + `& * ! | > ? [ ] { } % @`)。
- **daemon MetricsSink wiring**(task #79)— daemon 生命周期一条
  TaskRecord,finalize 触发点 = Shutdown 消息或 accept loop Err。
  单 client 断连不 finalize(下 client 续用同 record)。SIGKILL /
  OOM 丢 record 已文档化为 known limitation。
- **`config_hash` 真 sha256**(task #80)— TaskRecord 的 `config_hash`
  从 `""` 占位换成 `sha256:<64-hex>`,从 YAML bytes 算出。
- **CJK / 全角字符列对齐**(task #84)— `sage skill-score` 报表的
  `truncate_for_column` 和 padding 用 `unicode-width` 的 display
  width 而非 char count,中文 skill 名表格终于对齐。
- **Arc blanket impl 取代 6 个 newtype wrapper**(task #71)。
  `impl<T: ?Sized + Trait> Trait for Arc<T>` 配 5 个 hook trait +
  LlmProvider + AgentTool。
- **Agent `attach_session / detach_session` API**(task #86)。
  4 个分散 setter 收敛为一对原子操作。
- **agent_loop 的 orphan `generate_session_id` 清理**(task #87)。
  重命名为 `generate_loop_trace_id`;消费点优先用 agent 自己的
  session_id。
- **skill_install P2/P3 review 跟进** — `default_name_for` 对
  `.` / `..` 正确 canonicalize;`run_skill_add` 前置检查 agent 已
  初始化(防 typo 留垃圾 workspace 目录);fetch 失败(copy / git /
  npx)统一在入口 `remove_dir_all(dst)` 兜底回滚。

### 工具 / CI

- **provider_specs 黄金快照**(task #74 sub 4)— 锁死 v0.0.3 的 18
  个 provider 的 `(id, api_kind)` 二元组全集。漂移打印"去 pi-mono
  核对"三步修复指引。对齐 CLAUDE.md 强制要求。

### 测试 / 文档

- **+16 net new tests**(2229 → 2247)。并发 race、expanded YAML
  guard、serde_yaml roundtrip、daemon finalize Ok/Err/idempotent、
  provider snapshot、skill_evaluate backup/rollback、npm-style
  detect、dot-path resolve、mid-fetch rollback 等。

## [0.0.2] — 2026-04-17

### 架构落地

- **Skill 模型确立**。`workspace/skills/<name>/SKILL.md` 作为单一单位
  （对齐外部生态的 SKILL.md 约定），`workspace/skills/INDEX.md` 由 agent
  自维护；system prompt 不再预装 skill body，agent 按需用 Read 工具取。
  从前的 `workspace/craft/` 路径 + `/slash` 调用 + 自动注入 body 一并
  移除（task #88，-1891 LOC net）。
- **`sage skill add` / `sage skill list`** — 从 local path / git URL 装
  skill，自动更新 `INDEX.md`（task #82）。
- **`sage skill-score` / `sage skill-score --needs-evaluation`** —
  离线 skill token 效率评估（task #72 sub-path 3）。
- **CancellationToken 贯通** agent_loop / LLM 调用 / tool 执行。`sage chat`
  的 Ctrl+C 能真正中断在途 session，不再只在 readline 边界响应
  （task #69）。
- **ApiProviderRegistry 实例化**。每个 `SageEngine` 自持一份 provider
  注册表，两个 engine 并存不再通过 global 互相污染（task #70）。
- **MetricsCollector 接 AgentEvent 流**。UserDriven session 结束落盘
  `workspace/metrics/<ulid>.json` + `summary.json`（task #75）。

### CLI

- `sage run --dev` flag + `sandbox.mode: host` 配置项。没装 libkrunfw
  的机器也能跑所有 agent（task #76）。
- 所有 `--agent <name>` 入口统一走 `validate_agent_name`，拒绝 `../…` /
  绝对路径 / 空字符串。主 gate 在 `load_agent_config`，每个 pub entry
  额外 defense-in-depth（task #85）。
- Commands::CraftScore → Commands::SkillScore，`sage craft-score` →
  `sage skill-score`（task #88 一并 rename）。

### Runtime

- `SageSession::cancel()` / `is_cancelled()` / `cancel_token()` 新 API。
- `SageError::InvalidModel` 作 canonical format source — provider 4xx
  body 判定为 "model id 错" 时路由到带 `hint_docs_url` 的用户消息
  （Sprint 12 M2）。
- M2 keyword 策略收紧：`"does not exist"` 不再裸匹配，要求与 `"model"`
  共现；`quota_exceeded` / `permission_denied` 加入硬排除（task #77 (6)）。

### Observability

- Drop SessionEnd 的孤儿日志从 `warn!` 升级到 `error!`，携带 turn_count
  方便 debug（task #77 (4)）。
- `SessionType::CraftEvaluation` → `SessionType::SkillEvaluation`（统一
  术语，archive wire 格式同步更新 — 预发布阶段无历史包袱）。

### 已知缺口（v0.0.3 补）

- `sage skill add <npm-style>`（包装 `npx skills add`）
- daemon 自动触发 `SkillEvaluation` session（task #83）
- CJK / 全角字符下 `skill-score` 报表对齐（task #84）
- `TaskRecord.config_hash` 真算 sha256（当前留空 sentinel）
- `sage skill remove` / `sage skill update`（pull / 重装）
- Arc newtype blanket impl 重构（task #71）
- Agent.hook_bus Option → attach_session API 重构（task #86）

### 测试

workspace: ~2224 passed / 0 failed / 12 ignored

## [0.0.1] — internal milestone

- Agent 执行引擎（sage-runtime）、Host microVM SDK（sage-sandbox）、
  Guest agent（sage-guest）三件套初始实现。
- CLI / daemon / TUI / 触发器基础。
- 18 个 LLM Provider（Anthropic / OpenAI / Google / Bedrock / OpenAI-compat
  十余个）。
- MetricsCollector + TaskRecord schema。
- Sprint 5–12 的迭代落地（详见 docs/TODO.md）。
