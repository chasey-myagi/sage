# Changelog

所有值得记录的变更按倒序列在这里。版本遵循 semver，但在 v0.1.0 之前
不保证 API / CLI / 磁盘布局稳定。

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
