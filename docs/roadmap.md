# Roadmap

Sage 的定位是**领域专家 Agent 应用**：给一个领域配一个有独立工作空间、有记忆沉淀的专员。

不做：通用 coding agent（去用 Claude Code / omp）。不做：microVM 沙箱（直到有人真要跑不信任的代码）。不做：100 个 provider / 65 个主题 / 12 个 hook 事件 / wiki 系统。

## v0.4 — 飞书专员闭环

把"领域专员"从概念落到一个能 dogfood 的产物。

- [x] sage 工作目录读取 `CLAUDE.md` + `.sage/settings.json`
- [x] 飞书 workspace 雏形：`~/Dev/cc/feishu/`（CLAUDE.md + craft/ + .sage/settings.json + sage-feishu.sh）
- [x] **Fix 权限引擎**：`check_tool_permission_with_content` 让 content-level 规则（`Bash(lark-cli:*)`）真正生效，工具名匹配大小写不敏感（"Bash" 规则匹配 "bash" tool）。
- [x] **Kimi Code 接入**：作为 `kimi-code` provider（与 `kimi` 是两个产品），走 Anthropic-compat 协议，使用 `KIMI_CODE_API_KEY`。e2e 13/13 通过。顺带修两个 SSE parser 的 `data:` 空格 bug。
- [ ] **Anthropic-compat 路径 metrics 漏统**：`message_start.usage.input_tokens` 和 `message_delta.usage.output_tokens` 是两条分开的 SSE 事件，最终汇总到 `message.usage` 时只保留了 output。本地 e2e 在 kimi-code/anthropic 路径下 `tokens_input` 总显示 0。不影响功能；修需要 anthropic.rs 在流末把 message_start 的 usage merge 进 TurnEnd message。
- [ ] 跑通三个真实任务：
  - 查待面试日程并按时间排序
  - 新增公司 + 同步到飞书日历
  - 更新面试结果 + 写复盘
- [ ] 第一份"craft 沉淀"——agent 跑完一次新任务后把 SOP 写进 `craft/*.md`

## v0.5 — 分层记忆（GA L0–L4）

抄 GenericAgent 的姿态：把 long-context 砍掉，记忆放外部。

- [ ] 在 workspace 引入分层：`AGENT.md`（L0 元规则）+ `MEMORY.md`（L1 索引）+ `facts/`（L2 全局事实）+ `craft/*.md`（L3 SOP）+ `sessions/`（L4 归档）
- [ ] system_prompt 启动时自动注入 L0+L1，按需 read L2/L3
- [ ] `update_working_checkpoint` 工具：会话中段记 200-token 小本本
- [ ] 一次任务完成后归档为 L4 session

## v0.6 — Caster 注册（Rune 调度入口）

让飞书专员能被 Rune Runtime 远程拉起。

- [ ] sage 注册为 Caster
- [ ] 接受一次性任务调度（cron / webhook）
- [ ] 任务结果回报机制

## 长期不做（明确的反目标）

| 不做 | 替代 |
|---|---|
| microVM 沙箱 | 工具白名单 + approval（已有）|
| TUI 雕花（kill_ring/fuzzy/undo_stack/65 主题）| 当前够用，冻结 |
| 子代理 / Team / Worktree 隔离 | v1 不需要 |
| 14 个 provider | Anthropic + Google + OpenAI-compat（3 个）|
| 7000 行 model registry | 12 条手写 + `discover_models` |
| 通用 coding agent 模式 | 用 CC/omp |
| 抢"任意领域" | 一次只做一个领域，跑通再说 |

## 发布

详见 [release-process.md](release-process.md)。**版本号 = 构建 = SHA256，绝不 force-push 已发布 tag。**
