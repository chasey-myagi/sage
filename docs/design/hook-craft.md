# Hook + Skill/Craft 系统设计

## Hook 是代码层 Harness

Hook 不依赖 LLM 判断，是确定性执行的。这意味着：
- `PreToolUse` → 拦截危险操作，不管 LLM 怎么想
- `Stop` → 质量评估，不够好就 exit 2 让 Agent 继续
- `UserPromptSubmit` → 输入校验/增强，在 LLM 看到之前就处理

在 Runeforge 场景下，**Sage 的 Stop hook 就是 Harness eval 的入口**。Runeforge 定义质量标准（eval 脚本），通过 Sage hook 机制注入——这是 Harness 能控制 Sage 行为的那根线。

### 12 核心 Hook 事件

从 CC 的 27 个事件中精选，去掉多 Agent 协作类（SubagentStart/Stop、TeammateIdle、TaskCreated 等），增加 3 个 Sage 特有事件：

| Event | 优先级 | 用途 |
|-------|-------|------|
| `SessionStart` | P0 | 初始化环境、注入动态上下文（替代硬编码启动逻辑） |
| `UserPromptSubmit` | P0 | 拦截/增强用户输入（最常用） |
| `PreToolUse` | P0 | 工具调用前审计/安全防护（exit 2 = 阻断） |
| `PostToolUse` | P0 | 工具调用后回调/副作用触发 |
| `Stop` | P0 | Agent 结束前质量检查，exit 2 = 要求继续 |
| `SessionEnd` | P1 | 保存状态、清理外部资源 |
| `PreCompact` | P1 | 压缩前自定义处理（exit 2 = 阻断压缩） |
| `PostCompact` | P1 | 压缩后通知/记录 |
| `FileChanged` | P1 | 监听 workspace 文件变化（替代轮询） |
| `MemoryUpdated` *(Sage)* | P2 | MEMORY.md 写入后触发（知识沉淀回调） |
| `AgentSwitched` *(Sage)* | P2 | `/switch <agent>` 切换 Agent 时 |
| `WorkspaceChanged` *(Sage)* | P2 | 工作目录变更 |

### 4 种 Hook 类型

| 类型 | 执行方式 | 适用场景 |
|------|---------|---------|
| `command` | 宿主 shell 执行脚本（stdin JSON + exit code） | 80% — 审计/日志/拦截/注入 |
| `http` | POST JSON 到 webhook URL | 外部系统集成、Slack/飞书通知 |
| `prompt` | 调小模型语义评估 | 无法用 regex 判断的内容 |
| `agent` | 启动子 Sage 实例验证 | 需要工具调用的复杂验证 |

**Exit code 协议**（command hook）：
- `exit 0` — 成功，stdout 按事件语义决定是否注入模型
- `exit 2` — 阻断，stderr 注入模型，让模型感知被阻断的原因
- `exit N` — 用户可见错误（stderr 展示给用户），执行继续

### Hook 配置

```yaml
# ~/.sage/agents/feishu/agent.yaml（Agent 级）
hooks:
  SessionStart:
    - hooks:
        - type: command
          command: "python3 ~/.sage/agents/feishu/init.py"
  PreToolUse:
    - matcher: "bash"        # 只在 bash 工具时触发
      hooks:
        - type: command
          command: "bash ~/.sage/agents/feishu/hooks/audit.sh"
          if: "bash(curl *)" # 只在 curl 子命令时触发

# ~/.sage/hooks.yaml（全局，跨所有 Agent）
PreToolUse:
  - matcher: "bash"
    hooks:
      - type: command
        command: "~/.sage/global-hooks/security-audit.sh"
```

### 实现任务（v0.9.0 H1）

- [ ] `HookEvent` 枚举（12 个事件）+ tokio broadcast channel 广播系统
- [ ] command hook 执行引擎：fork shell 进程，写 stdin JSON，读 exit code + stdout/stderr
- [ ] exit 2 协议：阻断工具调用，将 stderr 作为 steering message 注入 agent loop
- [ ] `agent.yaml` 中 `hooks` 字段解析 + 全局 `~/.sage/hooks.yaml` 合并加载
- [ ] Matcher 过滤（tool name 前缀匹配）+ `if` 条件（权限规则语法子集）
- [ ] http hook：reqwest POST，body = stdin JSON 格式的 payload
- [ ] prompt/agent hook：预留接口，v0.9.x 实现

---

## Skill（全局）与 Craft（workspace）两级架构

**全局 Skills（`~/.sage/skills/`）**：开发者/领域专家编写，Agent 只读。这些是"教科书"——标准 Markdown SOP、API 参考模板等。平台维护，随发版更新。

**Craft（`workspace/craft/`）**：Agent 自管理的可复用产物工具箱。类型不限：

| 类型 | 示例 | 格式 |
|---|---|---|
| SOP / 操作手册 | 飞书日历批量更新步骤 | `CRAFT.md` + `metrics.json` |
| 可执行脚本 | 自动生成周报的 Python 脚本 | `.py` / `.sh` |
| 输出模板 | 日程摘要的 Markdown 格式 | `.md` |
| 素材 / 数据 | 生成的报表、配置文件 | `.csv` / `.json` / `.pdf` |

通过 `craft_manage` 工具创建/编辑/评估。

### 目录结构

```
~/.sage/
├── skills/                      # 全局 Skills（只读 Markdown）
│   ├── research-template.md
│   └── batch-update.md
└── agents/
    └── feishu/
        ├── agent.yaml
        └── workspace/
            └── craft/           # Agent Craft（可读写，类型任意）
                ├── batch-calendar-update/   # SOP 类
                │   ├── CRAFT.md             # Craft 正文
                │   └── metrics.json         # token 效率评分
                ├── weekly-report.py         # 脚本类
                └── summary-template.md      # 模板类
```

### SOP 类 Craft 文件格式

`CRAFT.md`（兼容 CC frontmatter 结构）：

```markdown
---
name: feishu-schedule
description: "批量更新飞书日历"
type: sop                        # sop / script / template / asset
whenToUse: "用户要批量操作多个日历事件时"
allowedTools: [bash, read, write]
agent: feishu
version: 3
---

# 批量日历更新

步骤：
1. 读取 /workspace/schedule.csv 中的事件列表
2. 逐条调用飞书日历 API 更新...
```

### Craft 生命周期

**创建触发条件**：
- 复杂任务成功完成（5+ 工具调用）
- 从错误中恢复的有效路径
- 用户纠正后的正确做法
- 反复出现的同类任务模式（3+ 次）

```
创建 → craft_manage(action: "create", name, type, content)
评估 → SOP 类每次使用记录 token 消耗到 metrics.json
优化 → 低效 SOP Craft 触发 CraftEvaluation 会话，Agent 自动重写
淘汰 → 评分持续低于阈值标记为 deprecated
```

### Token 效率评分

评分规则：`score = best_tokens / avg_tokens`。System prompt 中 SOP Craft 按 score 降序排列，评分越高的 Craft 推荐权重越高。

```json
// workspace/craft/<craft-name>/metrics.json
{
  "craft_name": "batch-calendar-update",
  "type": "sop",
  "executions": [
    { "ts": "2026-04-14T10:00:00Z", "task_hash": "abc123", "tokens_used": 1200, "success": true },
    { "ts": "2026-04-15T09:00:00Z", "task_hash": "abc123", "tokens_used": 980, "success": true }
  ],
  "avg_tokens": 1090,
  "best_tokens": 980,
  "score": 0.82,
  "version": 3
}
```

### 实现任务（v0.9.0 S1）

- [ ] 启动时扫描 `~/.sage/skills/`（全局 Skills）+ `workspace/craft/`（Agent Craft），SOP 类只加载 frontmatter（懒加载正文）
- [ ] 将 Skill + SOP Craft 列表注入 system_prompt（name + description + score，按 score 降序排列）
- [ ] TUI 中 `/skill-name [args]` 解析：加载完整正文，注入当前对话
- [ ] `$ARGUMENTS` 占位符替换
- [ ] Skill frontmatter 中 `hooks` 字段：激活时自动注册对应 hooks，退出时注销
- [ ] `agent` 过滤：有 `agent: feishu` 字段时只在对应 agent 会话中可见
- [ ] `craft_manage` 工具注册到 ToolRegistry：
  - `create(name, type, content, tags)` — 创建 Craft（SOP/脚本/模板）
  - `edit(name, content)` — 编辑（version++）
  - `delete(name)` — 删除
  - 全局 Skills 不可通过此工具修改
- [ ] Token 效率采集：PostToolUse hook 中记录 SOP Craft 使用的 token 数到 metrics.json
- [ ] 内置 skills（编译进二进制）：
  - `/memory` — 整理当前会话学到的内容，更新 memory/MEMORY.md
  - `/wiki` — 手动触发 wiki 维护，整理未处理的 session 记录
