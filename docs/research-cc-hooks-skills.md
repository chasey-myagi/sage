# CC Hook 机制与 Skill 系统研究报告

> 来源：`~/Dev/cc/external/claude-code/src/`
> 目的：为 Sage v0.7–v0.9 Hook & Skill 系统设计提供参考

---

## 一、CC Hook 系统

### 1.1 核心设计

Hook 是挂载在 Agent 生命周期特定事件上的**外部副作用**，通过标准 I/O + exit code 与 Agent 通信。Shell 优先，无需 SDK、无需编译。

```
Event 触发
  │
  ▼
Hook Runner（并发执行同 event 下所有匹配 Hook）
  │
  ├─ exit 0  → 成功（stdout 按事件语义决定是否注入模型）
  ├─ exit 2  → 阻断（stderr 注入模型，让模型处理）
  └─ exit N  → 用户可见错误（stderr 展示给用户，继续执行）
```

### 1.2 Hook Types（4种）

| 类型 | 配置 | 执行方式 | 适用场景 |
|------|------|---------|---------|
| `command` | `command: "bash script.sh"` | 在用户 shell 执行 | 80% 场景，脚本审计/日志/注入 |
| `http` | `url: "https://..."` | POST JSON 到 endpoint | Webhook 集成、外部系统通知 |
| `prompt` | `prompt: "验证..."` | 调小模型评估 | 语义判断（不适合 regex 的场景） |
| `agent` | `prompt: "验证单测通过..."` | 启动子 Agent 验证 | 复杂验证（需工具调用、多步推理） |

**command hook 的完整字段：**
```json
{
  "type": "command",
  "command": "python3 audit.py",
  "if": "Bash(git *)",       // 权限规则语法，只在匹配时触发
  "shell": "bash",           // bash | powershell
  "timeout": 30,             // 秒
  "statusMessage": "审计中...",
  "once": false,             // 执行一次后移除
  "async": false,            // 后台执行，不阻塞
  "asyncRewake": false       // 后台执行，exit 2 时唤醒模型
}
```

### 1.3 Hook Events（27个）

按 Agent 生命周期分组：

**会话级：**
| Event | 触发时机 | exit 2 效果 |
|-------|---------|-----------|
| `SessionStart` | 会话开始（startup/resume/clear/compact） | 忽略 |
| `SessionEnd` | 会话结束（clear/logout/exit） | 忽略 |
| `Setup` | init/maintenance 触发 | 忽略 |

**用户输入级：**
| Event | 触发时机 | exit 2 效果 |
|-------|---------|-----------|
| `UserPromptSubmit` | 用户发送消息 | 阻断 + 清除原始消息 |

**工具调用级：**
| Event | 触发时机 | exit 2 效果 |
|-------|---------|-----------|
| `PreToolUse` | 工具调用前 | 阻断工具调用，stderr → 模型 |
| `PostToolUse` | 工具调用后（成功） | stderr → 模型（立即继续） |
| `PostToolUseFailure` | 工具调用失败 | stderr → 模型 |
| `PermissionDenied` | auto 模式拒绝工具 | — |
| `PermissionRequest` | 权限对话框弹出 | 可返回 allow/deny 决策 |

**Agent 结束级：**
| Event | 触发时机 | exit 2 效果 |
|-------|---------|-----------|
| `Stop` | Agent 结束响应前 | 继续对话（注入 stderr → 模型） |
| `StopFailure` | API 错误结束 turn | fire-and-forget |

**压缩级：**
| Event | 触发时机 | exit 2 效果 |
|-------|---------|-----------|
| `PreCompact` | 压缩前 | 阻断压缩 |
| `PostCompact` | 压缩后 | — |

**文件系统级：**
| Event | 触发时机 | exit 2 效果 |
|-------|---------|-----------|
| `CwdChanged` | 工作目录变更 | — |
| `FileChanged` | 监听文件变化（matcher 指定文件名） | — |
| `InstructionsLoaded` | CLAUDE.md 等指令文件加载 | — |
| `ConfigChange` | 配置文件变更 | 阻断变更生效 |

**子 Agent 级（Swarm/Team 功能）：**
| Event | 触发时机 | exit 2 效果 |
|-------|---------|-----------|
| `SubagentStart` | 子 Agent 启动 | — |
| `SubagentStop` | 子 Agent 结束前 | 继续让子 Agent 运行 |
| `TeammateIdle` | Teammate 即将空闲 | 阻止 Teammate 进入 idle |
| `TaskCreated` | 任务创建 | 阻断任务创建 |
| `TaskCompleted` | 任务完成 | 阻断任务完成 |

**MCP 相关：**
`Elicitation`、`ElicitationResult`（MCP server 请求用户输入）

**Worktree 相关：**
`WorktreeCreate`、`WorktreeRemove`

### 1.4 Matcher 机制

每个 event 可设置 `matcher` 过滤，避免无谓触发：

```json
{
  "PreToolUse": [
    {
      "matcher": "Bash",      // 只在调用 bash 工具时触发
      "hooks": [{ "type": "command", "command": "audit.sh" }]
    },
    {
      "matcher": "Write",     // 只在写文件时触发
      "hooks": [{ "type": "command", "command": "backup.sh" }]
    }
  ]
}
```

`if` 字段提供更细粒度过滤（权限规则语法）：
```json
{ "type": "command", "command": "sh", "if": "Bash(git push*)" }
```

### 1.5 Hook 输入格式（stdin JSON）

```json
// PreToolUse
{ "tool_name": "Bash", "tool_input": { "command": "rm -rf /tmp" }, "tool_use_id": "..." }

// PostToolUse
{ "tool_name": "Bash", "tool_input": {...}, "tool_use_id": "...", "response": "..." }

// UserPromptSubmit
{ "prompt": "帮我查飞书日程", "session_id": "..." }

// SessionStart
{ "source": "startup", "session_id": "..." }

// Stop
{ "stop_reason": "end_turn", "session_id": "..." }
```

### 1.6 Hook 配置位置

```
settings.json（用户级）
  └── hooks: { "PreToolUse": [...], "PostToolUse": [...] }

.claude/settings.json（项目级）
  └── hooks: { ... }

Skill frontmatter（技能级，技能激活时自动注册）
  ---
  hooks:
    PostToolUse:
      - matcher: "Write"
        hooks:
          - type: command
            command: "lint.sh $ARGUMENTS"
  ---
```

---

## 二、CC Skill 系统

### 2.1 核心设计

Skill = **Markdown 文件 + YAML frontmatter**，由用户写，AI 读。`/skill-name` 调用时，skill 内容作为 prompt 注入当前对话。

**设计哲学**：
- 零编译（纯 Markdown）
- 用户可读可改（对比二进制插件）
- 按需加载（启动时只读 frontmatter，调用时才读全文）
- Skills 可嵌入 Hooks（skill 注册后自动激活其 hooks）

### 2.2 Skill 加载来源

```
优先级（高 → 低）：
  policy（企业管控）
    ├── ~/.claude/skills/           userSettings 级
    ├── .claude/skills/             projectSettings 级（当前项目）
    ├── plugin/skills/              插件提供的 skill
    ├── bundled（内置 skill）        编译进二进制
    └── mcp（MCP server 提供）

Deprecated: .claude/commands/（旧格式，仍支持）
```

### 2.3 Skill Frontmatter 完整字段

```markdown
---
name: my-skill                    # string，必填，= /command 名
description: "做某件事"            # string，必填，展示在 /skills 列表
whenToUse: "当...时调用"           # string，可选，提示 AI 何时自动调用
allowedTools: [Bash, Read]        # string[]，此 skill 可用的工具列表
argumentHint: "<file>"            # string，参数格式提示
version: "1.0.0"                  # string，版本号
model: "claude-haiku-4-5"        # string，覆盖默认模型
disableModelInvocation: false     # bool，true = 只注入 prompt 不调 LLM
userInvocable: true               # bool，false = 仅内部使用
context: inline                   # inline | fork（fork 开新对话）
paths: "src/**/*.ts"              # string，只在匹配路径时可见

# Skill 自带的 Hooks（skill 加载时自动注册）
hooks:
  PostToolUse:
    - matcher: "Write"
      hooks:
        - type: command
          command: "bash .hooks/lint.sh"
---

# Skill 正文（Markdown）

这里是注入给 Agent 的 prompt 内容...
```

### 2.4 Bundled Skill（编译进二进制）

内置 skills 通过 `registerBundledSkill()` 注册，支持携带参考文件：

```typescript
registerBundledSkill({
  name: 'remember',
  description: '整理记忆',
  whenToUse: '用户想整理记忆时',
  userInvocable: true,
  files: {
    'SKILL.md': '...',      // 首次调用时解压到临时目录
    'references/foo.md': '...',
  },
  async getPromptForCommand(args, ctx) {
    return [{ type: 'text', text: SKILL_PROMPT }]
  },
})
```

参考文件解压到 `~/.claude/skills-cache/<nonce>/<skill-name>/`，通过 `Base directory for this skill: <dir>` 告知 Agent 位置。

### 2.5 Skill 调用方式

```
TUI 中：
  /skill-name              # 无参数调用
  /skill-name arg1 arg2    # 带参数（$ARGUMENTS 占位符替换）

自动调用（whenToUse）：
  Agent 判断当前任务符合 whenToUse 描述时，自动选择 skill
```

### 2.6 Skill 的 Token 预算

- 启动时：只读 frontmatter（name + description + whenToUse），估算约 50-100 tokens/skill
- 调用时：完整内容注入，可能 1000-5000 tokens
- 系统提示中展示 skill 列表供 Agent 选择

---

## 三、给 Sage 的设计建议

### 3.1 Hook 系统：迁移最小可用集

Sage v0.7 不需要 27 个 events。按 Sage 产品形态，精选 **12 个核心 events**：

| Event | 迁移优先级 | Sage 特有价值 |
|-------|-----------|-------------|
| `SessionStart` | P0 | 每次 chat 启动时初始化环境、注入上下文 |
| `UserPromptSubmit` | P0 | 拦截/增强用户输入（最常用） |
| `PreToolUse` | P0 | 审计工具调用、安全防护 |
| `PostToolUse` | P0 | 结果回调、触发副作用 |
| `Stop` | P0 | Agent 结束前的质量检查，可要求继续 |
| `SessionEnd` | P1 | 会话结束时保存状态、清理资源 |
| `PreCompact` | P1 | 压缩前自定义指令 |
| `PostCompact` | P1 | 压缩后通知/记录 |
| `FileChanged` | P1 | 监听 workspace 文件变化（替代轮询） |
| `MemoryUpdated` | P2（Sage 特有） | MEMORY.md 被更新时触发（知识沉淀 Hook） |
| `AgentSwitched` | P2（Sage 特有） | 切换 Agent 时（`/switch feishu`） |
| `WorkspaceChanged` | P2（Sage 特有） | 工作目录变更 |

**关键差异**：Sage 是单用户产品，不需要 SubagentStart/Stop、TeammateIdle、TaskCreated/Completed 这类多 Agent 协作 events。

### 3.2 Hook 配置位置（Sage 版）

```yaml
# ~/.sage/agents/feishu/agent.yaml（Agent 级 Hooks）
hooks:
  SessionStart:
    - hooks:
        - type: command
          command: "python3 ~/.sage/agents/feishu/init.py"
  PreToolUse:
    - matcher: "bash"
      hooks:
        - type: command
          command: "bash ~/.sage/agents/feishu/hooks/pre-bash.sh"
          if: "bash(curl *)"   # 只在 curl 调用时触发

# ~/.sage/hooks.yaml（全局 Hooks，跨所有 Agent）
PreToolUse:
  - matcher: "bash"
    hooks:
      - type: command
        command: "~/.sage/global-hooks/audit.sh"
```

### 3.3 Skill 系统：Sage 版 Skill = Agent 的"专项能力模块"

Sage 的 Skill 和 CC 一样是 Markdown + frontmatter，但有 Sage 特定的扩展：

```markdown
---
name: feishu-schedule           # /feishu-schedule 调用
description: "批量更新飞书日历"
whenToUse: "用户要批量操作多个日历事件时"
allowedTools: [bash, read, write]
agent: feishu                   # Sage 特有：只在 feishu agent 中可用
---

# 批量日历更新

步骤：
1. 读取 /workspace/schedule.csv 中的事件列表
2. 逐条调用飞书 API 更新...
```

**Sage Skill 目录结构：**
```
~/.sage/
├── skills/                     # 全局 Skills（所有 Agent 可用）
│   ├── research-template.md
│   └── batch-update.md
└── agents/
    └── feishu/
        ├── agent.yaml
        ├── workspace/
        └── skills/             # 此 Agent 专属 Skills
            ├── schedule-batch.md
            └── doc-format.md
```

### 3.4 Hook × Skill 联动（最有价值的组合）

Skill frontmatter 可以嵌入 hooks，这样 `/skill-name` 激活后自动注册其 hooks：

```markdown
---
name: monitor-feishu-doc
description: "持续监听飞书文档变化"
hooks:
  FileChanged:
    - matcher: "*.md"
      hooks:
        - type: command
          command: "python3 ~/.sage/agents/feishu/skills/sync-doc.py $ARGUMENTS"
---

# 飞书文档监控模式

激活后，Sage 会监听 workspace 中的 .md 文件变化，自动同步到飞书...
```

### 3.5 与 CC 的关键差异

| 维度 | CC | Sage |
|------|-----|------|
| Hook 存储 | `settings.json`（JSON） | `agent.yaml`（YAML）+ 全局 `hooks.yaml` |
| Skill 存储 | `~/.claude/skills/`（全局） | `~/.sage/skills/`（全局）+ `~/.sage/agents/<name>/skills/`（per-agent） |
| Hook 事件数 | 27 个 | 12 个（精简，Sage 产品形态需要的） |
| Skill 调用 | `/skill-name` | `/skill-name` （完全相同） |
| Agent Hook | 验证工具（子 Agent） | 同：Sage 内部再起一个 sage 实例验证 |
| Skill 注册 | 编译时（bundled）/ 文件扫描 | 同：内置 skill + `~/.sage/skills/` 扫描 |

---

## 四、参考源码位置

| 主题 | 文件 |
|------|------|
| Hook Events 完整列表 | `src/utils/hooks/hooksConfigManager.ts` |
| Hook 类型 Schema | `src/schemas/hooks.ts` |
| Shell Hook 执行 | `src/utils/hooks/execAgentHook.ts` |
| Skill 加载机制 | `src/skills/loadSkillsDir.ts` |
| Skill 注册（内置） | `src/skills/bundledSkills.ts` |
| Skill 示例（remember） | `src/skills/bundled/remember.ts` |
| Hook 事件广播 | `src/utils/hooks/hookEvents.ts` |
