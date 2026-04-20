# Claude Code 完整知识手册

## 目录

**第一部分：内置工具**
1. [文件操作工具](#1-文件操作工具)
2. [搜索工具](#2-搜索工具)
3. [终端工具](#3-终端工具)
4. [交互工具](#4-交互工具)
5. [规划工具](#5-规划工具)
6. [任务管理工具](#6-任务管理工具)
7. [Agent 协作工具](#7-agent-协作工具)
8. [网络工具](#8-网络工具)
9. [Notebook 工具](#9-notebook-工具)
10. [Git Worktree 工具](#10-git-worktree-工具)
11. [Skill 工具](#11-skill-工具)
12. [MCP 外部工具](#12-mcp-外部工具)

**第二部分：工作流程与机制**
13. [Plan Mode 完整流程](#13-plan-mode-完整流程)
14. [Git Commit 工作流程](#14-git-commit-工作流程)
15. [Pull Request 工作流程](#15-pull-request-工作流程)
16. [多 Agent 团队协作流程](#16-多-agent-团队协作流程)
17. [权限与安全机制](#17-权限与安全机制)
18. [核心行为准则](#18-核心行为准则)
19. [工具使用优先级](#19-工具使用优先级)
20. [Auto Memory 机制](#20-auto-memory-机制)

---

# 第一部分：内置工具

## 1. 文件操作工具

### 1.1 Read - 读取文件

**用途**：读取本地文件系统中的文件内容。支持文本文件、图片（PNG/JPG 等）、PDF、Jupyter Notebook (.ipynb)。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `file_path` | string | 是 | 文件的绝对路径 |
| `offset` | number | 否 | 从第几行开始读取（适用于大文件） |
| `limit` | number | 否 | 读取的行数（适用于大文件） |
| `pages` | string | 否 | PDF 页码范围，如 `"1-5"`（仅 PDF 适用，大 PDF 必须指定） |

**输出**：文件内容，使用 `cat -n` 格式（带行号，从 1 开始）。每行超过 2000 字符会被截断。

**限制**：
- 默认读取前 2000 行
- 只能读文件，不能读目录（读目录用 `Bash` 的 `ls`）
- PDF 每次最多读 20 页
- 图片会以视觉方式呈现（多模态能力）

---

### 1.2 Edit - 编辑文件

**用途**：通过精确字符串替换来修改文件内容。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `file_path` | string | 是 | 文件的绝对路径 |
| `old_string` | string | 是 | 要被替换的文本（必须在文件中唯一） |
| `new_string` | string | 是 | 替换后的文本（必须与 old_string 不同） |
| `replace_all` | boolean | 否 | 是否替换所有匹配项（默认 `false`） |

**输出**：编辑成功或失败的提示信息。

**限制**：
- 必须先用 `Read` 读取过文件才能编辑
- `old_string` 必须在文件中唯一，否则需要提供更多上下文让它唯一，或使用 `replace_all`
- 必须保持原文件的缩进格式

---

### 1.3 Write - 写入文件

**用途**：将内容写入文件（会覆盖已有文件）。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `file_path` | string | 是 | 文件的绝对路径 |
| `content` | string | 是 | 要写入的内容 |

**输出**：写入成功或失败的提示信息。

**限制**：
- 如果文件已存在，必须先用 `Read` 读取
- 优先使用 `Edit` 修改已有文件，而非用 `Write` 覆盖
- 不会主动创建文档/README 文件，除非用户明确要求

---

## 2. 搜索工具

### 2.1 Glob - 文件名模式匹配

**用途**：通过 glob 模式快速查找文件，按修改时间排序返回。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `pattern` | string | 是 | glob 模式，如 `"**/*.ts"`, `"src/**/*.py"` |
| `path` | string | 否 | 搜索目录（默认当前工作目录） |

**输出**：匹配的文件路径列表，按修改时间排序。

**示例模式**：
- `"**/*.js"` - 递归查找所有 JS 文件
- `"src/components/**/*.tsx"` - 查找 src/components 下所有 TSX 文件
- `"**/test_*.py"` - 查找所有测试文件

---

### 2.2 Grep - 文件内容搜索

**用途**：基于 ripgrep 的正则表达式内容搜索工具。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `pattern` | string | 是 | 正则表达式模式 |
| `path` | string | 否 | 搜索路径（默认当前工作目录） |
| `glob` | string | 否 | 文件过滤 glob，如 `"*.js"`, `"*.{ts,tsx}"` |
| `type` | string | 否 | 文件类型过滤，如 `"js"`, `"py"`, `"rust"` |
| `output_mode` | string | 否 | `"files_with_matches"`（默认，仅文件路径）/ `"content"`（匹配行）/ `"count"`（计数） |
| `-i` | boolean | 否 | 忽略大小写 |
| `-n` | boolean | 否 | 显示行号（`content` 模式下默认 `true`） |
| `-A` | number | 否 | 匹配行之后显示的行数 |
| `-B` | number | 否 | 匹配行之前显示的行数 |
| `-C` / `context` | number | 否 | 匹配行前后显示的行数 |
| `head_limit` | number | 否 | 限制输出条数 |
| `offset` | number | 否 | 跳过前 N 条结果 |
| `multiline` | boolean | 否 | 多行匹配模式（默认 `false`） |

**输出**：根据 `output_mode` 返回匹配的文件路径、匹配行内容或匹配计数。

**示例**：
- 搜索函数定义：`pattern: "def process_data"`, `type: "py"`
- 搜索带上下文：`pattern: "TODO"`, `output_mode: "content"`, `-C: 2`

---

## 3. 终端工具

### 3.1 Bash - 执行命令

**用途**：执行 bash 命令，用于 git 操作、包管理、系统命令等终端操作。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `command` | string | 是 | 要执行的命令 |
| `description` | string | 否 | 命令的简短描述 |
| `timeout` | number | 否 | 超时（毫秒），默认 120000（2 分钟），最大 600000（10 分钟） |
| `run_in_background` | boolean | 否 | 后台运行，返回 task_id 供后续查看 |

**输出**：命令的标准输出和标准错误。超过 30000 字符会被截断。

**使用原则**：
- 不要用 Bash 代替专用工具（如用 `cat` 代替 `Read`、用 `grep` 代替 `Grep`）
- 主要用于：git 命令、npm/pip/uv 等包管理、docker 操作、运行测试、编译构建等
- 包含空格的路径需要用双引号包裹
- 多个独立命令可以并行调用；有依赖关系的命令用 `&&` 串联

---

## 4. 交互工具

### 4.1 AskUserQuestion - 向用户提问

**用途**：在执行过程中向用户提问以获取信息、澄清需求或让用户做选择。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `questions` | array | 是 | 问题列表（1-4 个问题） |

每个问题的结构：

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `question` | string | 是 | 问题内容 |
| `header` | string | 是 | 短标签（最多 12 字符） |
| `options` | array | 是 | 选项列表（2-4 个），每个包含 `label` 和 `description` |
| `multiSelect` | boolean | 是 | 是否允许多选 |

选项中可选的 `markdown` 字段可用于展示 ASCII 模型、代码片段等预览内容。

**输出**：用户选择的答案。用户总是可以选择 "Other" 自定义输入。

---

## 5. 规划工具

### 5.1 EnterPlanMode - 进入规划模式

**用途**：在开始非简单任务的实现之前，进入规划模式来探索代码库并设计实现方案，供用户审批。

**输入参数**：无。

**输出**：进入规划模式的确认。

**适用场景**：
- 新功能实现
- 多种可行方案需要选择
- 修改影响现有行为
- 架构决策
- 多文件变更
- 需求不明确需要先探索

**不适用场景**：
- 单行/几行的简单修复
- 用户给出了非常具体的指令
- 纯研究/探索任务

---

### 5.2 ExitPlanMode - 退出规划模式

**用途**：在规划模式中完成计划编写后，提交计划供用户审批。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `allowedPrompts` | array | 否 | 实现计划所需的权限描述列表，每项包含 `tool`（如 `"Bash"`）和 `prompt`（语义描述，如 `"run tests"`） |

**输出**：用户对计划的审批结果。

---

## 6. 任务管理工具

### 6.1 TaskCreate - 创建任务

**用途**：创建结构化的任务列表，用于跟踪复杂多步骤任务的进度。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `subject` | string | 是 | 任务标题（祈使句，如 "Fix authentication bug"） |
| `description` | string | 是 | 详细描述 |
| `activeForm` | string | 否 | 进行中时的显示文本（现在进行时，如 "Fixing authentication bug"） |
| `metadata` | object | 否 | 附加元数据 |

**输出**：创建的任务 ID 和详情。新任务状态为 `pending`。

---

### 6.2 TaskGet - 获取任务详情

**用途**：通过 ID 获取任务的完整详情。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `taskId` | string | 是 | 任务 ID |

**输出**：任务的 subject、description、status、blocks、blockedBy 等完整信息。

---

### 6.3 TaskUpdate - 更新任务

**用途**：更新任务的状态、描述、所有者、依赖关系等。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `taskId` | string | 是 | 任务 ID |
| `status` | string | 否 | `"pending"` / `"in_progress"` / `"completed"` / `"deleted"` |
| `subject` | string | 否 | 新标题 |
| `description` | string | 否 | 新描述 |
| `activeForm` | string | 否 | 进行中显示文本 |
| `owner` | string | 否 | 任务负责人 |
| `metadata` | object | 否 | 合并元数据（设为 null 可删除某个 key） |
| `addBlocks` | array | 否 | 本任务阻塞的任务 ID 列表 |
| `addBlockedBy` | array | 否 | 阻塞本任务的任务 ID 列表 |

**输出**：更新后的任务详情。

---

### 6.4 TaskList - 列出所有任务

**用途**：查看所有任务的摘要列表。

**输入参数**：无。

**输出**：所有任务的 id、subject、status、owner、blockedBy 信息。

---

## 7. Agent 协作工具

### 7.1 Task (Agent) - 启动子 Agent

**用途**：启动专门的子 Agent 来自主处理复杂的多步骤任务。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `prompt` | string | 是 | 任务描述 |
| `description` | string | 是 | 简短描述（3-5 个词） |
| `subagent_type` | string | 是 | Agent 类型（见下表） |
| `model` | string | 否 | 使用的模型：`"sonnet"` / `"opus"` / `"haiku"` |
| `mode` | string | 否 | 权限模式：`"acceptEdits"` / `"bypassPermissions"` / `"default"` / `"dontAsk"` / `"plan"` |
| `isolation` | string | 否 | `"worktree"` 在隔离的 git worktree 中运行 |
| `run_in_background` | boolean | 否 | 后台运行 |
| `resume` | string | 否 | 恢复之前的 Agent（传入 agent ID） |
| `team_name` | string | 否 | 团队名称 |
| `name` | string | 否 | Agent 名称 |
| `max_turns` | number | 否 | 最大轮次 |

**可用的 Agent 类型**：

| 类型 | 说明 | 可用工具 |
|------|------|----------|
| `Bash` | 命令执行专家 | 仅 Bash |
| `general-purpose` | 通用 Agent，搜索代码、执行多步任务 | 所有工具 |
| `Explore` | 快速探索代码库（只读） | 除 Task/Edit/Write/NotebookEdit 外的所有工具 |
| `Plan` | 软件架构师，设计实现方案（只读） | 除 Task/Edit/Write/NotebookEdit 外的所有工具 |
| `code-simplifier` | 简化和优化代码 | 所有工具 |
| `statusline-setup` | 配置状态栏 | Read, Edit |
| `claude-code-guide` | Claude Code 使用指南问答 | Glob, Grep, Read, WebFetch, WebSearch |

**输出**：Agent 执行完成后返回结果消息和 agent ID（可用于 `resume` 恢复）。

---

### 7.2 TaskOutput - 获取后台任务输出

**用途**：获取运行中或已完成的后台任务的输出。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `task_id` | string | 是 | 任务 ID |
| `block` | boolean | 是 | 是否阻塞等待完成（默认 `true`） |
| `timeout` | number | 是 | 最大等待时间（毫秒），默认 30000，最大 600000 |

**输出**：任务输出和状态信息。

---

### 7.3 TaskStop - 停止后台任务

**用途**：停止正在运行的后台任务。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `task_id` | string | 否 | 要停止的任务 ID |

**输出**：成功或失败状态。

---

### 7.4 TeamCreate - 创建团队

**用途**：创建多 Agent 团队来协作完成项目。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `team_name` | string | 是 | 团队名称 |
| `description` | string | 否 | 团队描述 |
| `agent_type` | string | 否 | 团队负责人的角色类型 |

**输出**：创建团队配置文件（`~/.claude/teams/{team-name}.json`）和对应的任务列表目录（`~/.claude/tasks/{team-name}/`）。

---

### 7.5 TeamDelete - 删除团队

**用途**：删除团队和相关任务目录（所有成员必须先关闭）。

**输入参数**：无。

**输出**：删除成功或失败状态。

---

### 7.6 SendMessage - 发送消息

**用途**：在团队中向队友发送消息、广播、关闭请求等。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `type` | string | 是 | 消息类型（见下表） |
| `recipient` | string | 条件 | 接收者名称（`message`、`shutdown_request`、`plan_approval_response` 必填） |
| `content` | string | 否 | 消息内容 |
| `summary` | string | 条件 | 5-10 词摘要（`message` 和 `broadcast` 必填） |
| `request_id` | string | 条件 | 请求 ID（`shutdown_response` 和 `plan_approval_response` 必填） |
| `approve` | boolean | 条件 | 是否批准（`shutdown_response` 和 `plan_approval_response` 必填） |

**消息类型**：

| type | 说明 |
|------|------|
| `message` | 发送私信给指定队友 |
| `broadcast` | 广播消息给所有队友（慎用，开销大，每人一条） |
| `shutdown_request` | 请求队友关闭 |
| `shutdown_response` | 响应关闭请求 |
| `plan_approval_response` | 批准/拒绝队友的计划 |

---

## 8. 网络工具

### 8.1 WebSearch - 网络搜索

**用途**：搜索互联网获取最新信息。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `query` | string | 是 | 搜索查询（最少 2 个字符） |
| `allowed_domains` | array | 否 | 仅包含这些域名的结果 |
| `blocked_domains` | array | 否 | 排除这些域名的结果 |

**输出**：搜索结果，包括链接（markdown 超链接格式）。回答后必须附上 Sources 部分。

---

### 8.2 WebFetch - 获取网页内容

**用途**：获取指定 URL 的内容并用 AI 处理分析。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `url` | string | 是 | 完整 URL |
| `prompt` | string | 是 | 对获取内容的处理提示词 |

**输出**：AI 处理后的内容摘要。

**限制**：
- 不能访问需要认证的 URL（Google Docs、Jira 等）
- HTTP 自动升级为 HTTPS
- 内容过大时会被摘要
- 有 15 分钟缓存
- 重定向到不同域名时会返回重定向 URL，需要重新请求

---

## 9. Notebook 工具

### 9.1 NotebookEdit - 编辑 Jupyter Notebook

**用途**：替换、插入或删除 Jupyter Notebook (.ipynb) 中的特定单元格。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `notebook_path` | string | 是 | Notebook 的绝对路径 |
| `new_source` | string | 是 | 新的单元格源代码 |
| `cell_id` | string | 否 | 目标单元格 ID（插入时，新单元格插入到此 ID 之后） |
| `cell_type` | string | 否 | 单元格类型：`"code"` / `"markdown"`（插入时必填） |
| `edit_mode` | string | 否 | 编辑模式：`"replace"`（默认）/ `"insert"` / `"delete"` |

**输出**：编辑成功或失败的提示。

---

## 10. Git Worktree 工具

### 10.1 EnterWorktree - 创建 Git Worktree

**用途**：创建隔离的 git worktree，在独立副本中工作。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | string | 否 | worktree 名称（不提供则随机生成） |

**输出**：切换到新 worktree 的确认信息。

**行为**：
- 在 git 仓库中：在 `.claude/worktrees/` 下创建新 worktree，基于 HEAD 创建新分支
- 会话退出时，用户会被询问是否保留 worktree

**限制**：
- 必须在 git 仓库中（或有配置 hooks）
- 不能已在 worktree 中
- **只有用户明确说 "worktree" 时才使用**

---

## 11. Skill 工具

### 11.1 Skill - 执行技能

**用途**：执行用户配置的 Skill（对应用户输入的 `/斜杠命令`）。

**输入参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `skill` | string | 是 | 技能名称，如 `"commit"`, `"review-pr"` |
| `args` | string | 否 | 技能参数 |

**输出**：技能执行的结果。

**当前可用技能**：
- `keybindings-help` - 自定义键盘快捷键
- `code-review` - 代码审查 PR
- `frontend-design` - 创建前端界面

---

## 12. MCP 外部工具

以下是通过 MCP（Model Context Protocol）服务器提供的外部工具，非 Claude Code 内置，但在当前环境中可用。

### 12.1 Claude in Chrome - 浏览器自动化

提供 Chrome 浏览器的自动化操作能力。

| 工具 | 用途 |
|------|------|
| `tabs_context_mcp` | 获取当前 MCP tab 组的上下文信息（每次会话必须先调用） |
| `tabs_create_mcp` | 在 MCP tab 组中创建新标签页 |
| `navigate` | 导航到 URL 或前进/后退 |
| `read_page` | 获取页面可访问性树（元素结构），支持 `filter`（`"interactive"` / `"all"`） |
| `find` | 用自然语言查找页面元素（如 "search bar"、"login button"） |
| `computer` | 鼠标/键盘操作和截图（`left_click`/`type`/`screenshot`/`scroll`/`key`/`zoom` 等） |
| `javascript_tool` | 在页面上下文中执行 JavaScript |
| `form_input` | 通过 ref ID 设置表单元素的值 |
| `get_page_text` | 提取页面纯文本内容 |
| `upload_image` | 上传图片到页面（支持 ref 或坐标拖拽） |
| `resize_window` | 调整浏览器窗口大小 |
| `gif_creator` | 录制/导出 GIF 动画（`start_recording`/`stop_recording`/`export`/`clear`） |
| `read_console_messages` | 读取浏览器控制台消息（支持 pattern 过滤） |
| `read_network_requests` | 读取网络请求（支持 urlPattern 过滤） |
| `shortcuts_list` | 列出可用快捷方式 |
| `shortcuts_execute` | 执行快捷方式 |
| `switch_browser` | 切换连接的 Chrome 浏览器 |
| `update_plan` | 向用户展示操作计划供审批（包含 domains 和 approach） |

### 12.2 Context7 - 文档查询

提供编程库/框架的最新文档和代码示例查询。

| 工具 | 用途 |
|------|------|
| `resolve-library-id` | 将库名解析为 Context7 兼容的库 ID（必须先调用） |
| `query-docs` | 根据库 ID 查询文档和代码示例 |

**使用流程**：先 `resolve-library-id` 获取库 ID，再 `query-docs` 查询文档。每个问题最多各调用 3 次。

---

# 第二部分：工作流程与机制

## 13. Plan Mode 完整流程

### 13.1 什么是 Plan Mode

Plan Mode 是 Claude Code 的一种特殊工作模式，核心理念是**先规划，后实现**。在此模式下，Claude 会先深入探索代码库、理解现有架构、设计实现方案，然后将方案提交给用户审批。只有用户批准后才进入实际编码阶段。

### 13.2 触发条件

Claude 会在以下情况**主动**调用 `EnterPlanMode` 进入规划模式：

| 条件 | 示例 |
|------|------|
| 新功能实现 | "添加一个登出按钮" — 需要确定位置、点击行为等 |
| 多种可行方案 | "给 API 添加缓存" — Redis vs 内存 vs 文件 |
| 修改已有行为 | "更新登录流程" — 需确认变更范围 |
| 架构决策 | "添加实时更新" — WebSocket vs SSE vs 轮询 |
| 多文件变更 | "重构认证系统" — 涉及多个文件 |
| 需求不明确 | "让应用更快" — 需先 profile 找瓶颈 |
| 用户偏好很重要 | 实现可以有多种合理方向 |

以下情况**不进入** Plan Mode：

| 条件 | 示例 |
|------|------|
| 简单修复 | 修复 typo、明显 bug |
| 用户指令非常具体 | 用户已给出详细的实现步骤 |
| 纯研究/探索任务 | "哪些文件处理路由？" |

### 13.3 完整工作流程

```
用户提出任务
    │
    ▼
┌─────────────────────┐
│ 1. 判断是否需要规划  │ ── 简单任务 ──→ 直接实现
│    (EnterPlanMode)   │
└─────────┬───────────┘
          │ 需要规划
          ▼
┌─────────────────────┐
│ 2. 进入规划模式      │  用户需同意进入
└─────────┬───────────┘
          │
          ▼
┌─────────────────────────────────────────┐
│ 3. 探索阶段（只读，不修改任何文件）       │
│                                         │
│  可用工具：                              │
│  - Glob：查找文件                        │
│  - Grep：搜索代码内容                    │
│  - Read：阅读文件                        │
│  - Bash：执行只读命令（git log 等）       │
│  - WebSearch / WebFetch：查阅文档        │
│  - AskUserQuestion：向用户澄清需求       │
│                                         │
│  不可用工具（被禁用）：                    │
│  - Edit：不能编辑文件                     │
│  - Write：不能写入文件                    │
│  - NotebookEdit：不能编辑 notebook        │
│  - Task：不能启动子 Agent                 │
└─────────┬───────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────┐
│ 4. 编写计划                              │
│                                         │
│  计划内容通常包括：                        │
│  - 需要修改的文件清单                     │
│  - 每个文件的具体变更描述                  │
│  - 实现步骤和顺序                         │
│  - 架构选择和权衡分析                     │
│  - 可能的风险和注意事项                    │
│                                         │
│  计划写入到系统指定的计划文件中             │
└─────────┬───────────────────────────────┘
          │
          ▼
┌─────────────────────┐
│ 5. 提交计划审批      │
│   (ExitPlanMode)     │
│                     │
│  可指定 allowedPrompts：                │
│  实现时需要的权限列表                     │
│  如 [{tool:"Bash",                     │
│       prompt:"run tests"}]             │
└─────────┬───────────┘
          │
          ▼
┌─────────────────────┐
│ 6. 用户审批          │
│                     │
│  ├─ 批准 → 进入实现  │
│  ├─ 拒绝 → 修改计划  │
│  └─ 反馈 → 调整方向  │
└─────────┬───────────┘
          │ 批准
          ▼
┌─────────────────────────────────────────┐
│ 7. 实现阶段                              │
│                                         │
│  按照批准的计划逐步实现                    │
│  所有工具恢复可用                         │
└─────────────────────────────────────────┘
```

### 13.4 Plan Mode 中的注意事项

1. **AskUserQuestion vs ExitPlanMode**：
   - 需要澄清需求或选择方案时 → 用 `AskUserQuestion`
   - 计划已完成、准备提交审批时 → 用 `ExitPlanMode`
   - 不要用 `AskUserQuestion` 问 "计划可以吗？" — 这正是 `ExitPlanMode` 的用途

2. **用户在审批前看不到计划**：不要在 `AskUserQuestion` 中引用 "计划"，因为用户只有在 `ExitPlanMode` 后才能看到计划内容

3. **纯研究任务不需要 ExitPlanMode**：如果只是搜索文件、阅读代码、理解架构，不涉及后续编码实现，则不需要调用 `ExitPlanMode`

### 13.5 团队中的 Plan Mode

当子 Agent 以 `mode: "plan"` 启动时：
- 子 Agent 会自动进入 Plan Mode
- 子 Agent 调用 `ExitPlanMode` 时，会向团队负责人发送 `plan_approval_request`
- 团队负责人通过 `SendMessage` 的 `plan_approval_response` 类型来批准/拒绝
- 批准后子 Agent 自动退出 Plan Mode，可以开始编辑文件

---

## 14. Git Commit 工作流程

**触发条件**：用户明确要求创建 commit 时才执行，不主动 commit。

### 完整流程

```
1. 信息收集（并行执行）
   ├─ git status          查看未跟踪文件（不用 -uall）
   ├─ git diff            查看暂存和未暂存的变更
   └─ git log             查看近期 commit 消息风格

2. 分析与起草
   ├─ 判断变更性质（新功能 / 增强 / 修复 / 重构 / 测试 / 文档）
   ├─ 检查是否包含敏感文件（.env, credentials 等）
   └─ 起草 1-2 句 commit 消息，聚焦于 "why" 而非 "what"

3. 执行（并行执行）
   ├─ git add <具体文件>   添加相关文件（不用 git add -A 或 .）
   └─ git commit           提交，消息结尾附带：
                           Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>

4. 验证
   └─ git status          确认提交成功
```

### 安全规则

| 规则 | 说明 |
|------|------|
| 不修改 git config | 永不更改 git 配置 |
| 不执行破坏性命令 | `push --force`, `reset --hard`, `checkout .`, `clean -f` 等需用户明确要求 |
| 不跳过 hooks | 不用 `--no-verify`，除非用户要求 |
| 不 amend | 总是创建新 commit，除非用户明确要求 amend |
| 不 push | 不主动推送到远程，除非用户明确要求 |
| 不用交互模式 | 不用 `git rebase -i` 或 `git add -i`（不支持交互输入） |
| commit 消息格式 | 通过 HEREDOC 传递，确保格式正确 |

### Commit 消息格式示例

```bash
git commit -m "$(cat <<'EOF'
Fix user login race condition when session expires

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## 15. Pull Request 工作流程

### 完整流程

```
1. 信息收集（并行执行）
   ├─ git status                        查看未跟踪文件
   ├─ git diff                          查看未提交的变更
   ├─ 检查远程分支跟踪状态                是否需要 push
   └─ git log + git diff base...HEAD    查看所有 commit 历史

2. 分析（查看所有 commit，不仅仅是最新的！）
   ├─ PR 标题：简短（<70 字符）
   └─ PR 描述：详细说明

3. 执行（并行执行）
   ├─ 创建分支（如需要）
   ├─ git push -u（如需要）
   └─ gh pr create
```

### PR 格式模板

```bash
gh pr create --title "the pr title" --body "$(cat <<'EOF'
## Summary
<1-3 bullet points>

## Test plan
[Bulleted markdown checklist of TODOs for testing...]

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## 16. 多 Agent 团队协作流程

### 完整生命周期

```
1. 创建团队 (TeamCreate)
   └─ 生成 ~/.claude/teams/{name}/config.json
   └─ 生成 ~/.claude/tasks/{name}/ 任务目录

2. 创建任务 (TaskCreate)
   └─ 任务自动关联到团队的任务列表

3. 启动队友 (Task 工具，指定 team_name 和 name)
   └─ 队友加入团队，可以看到共享任务列表

4. 分配任务 (TaskUpdate 设置 owner)
   └─ 队友认领或被分配任务

5. 队友执行
   ├─ 查看任务 (TaskList/TaskGet)
   ├─ 标记进行中 (TaskUpdate status: in_progress)
   ├─ 完成后标记 (TaskUpdate status: completed)
   ├─ 发消息沟通 (SendMessage)
   └─ 每轮结束后自动进入 idle 状态（正常行为）

6. 关闭团队
   ├─ 向所有队友发送 shutdown_request
   ├─ 等待队友 shutdown_response 确认
   └─ TeamDelete 清理资源
```

### 关键概念

- **队友发现**：读取 `~/.claude/teams/{name}/config.json` 获取成员列表
- **始终用名称通信**：用 `name` 而非 `agentId` 发消息
- **Idle 是正常状态**：队友每轮结束后自动 idle，发消息即可唤醒
- **消息自动投递**：无需手动检查收件箱
- **任务按 ID 顺序优先**：多个可用任务时，先做 ID 小的

---

## 17. 权限与安全机制

### 17.1 操作风险分级

| 风险级别 | 说明 | 处理方式 |
|----------|------|----------|
| 低风险 | 读取文件、搜索代码、运行测试 | 可直接执行 |
| 中风险 | 编辑文件、创建文件 | 根据用户权限模式决定 |
| 高风险 | 删除文件/分支、force push、重置 | 必须向用户确认 |

### 17.2 破坏性操作清单

以下操作**必须**用户明确要求才能执行：

- `git push --force` / `git reset --hard` / `git checkout .`
- `git clean -f` / `git branch -D`
- `rm -rf`
- 覆盖未提交的变更
- 修改 CI/CD 配置
- 删除数据库表

### 17.3 高风险操作确认原则

需要确认的操作特征：
- **破坏性**：删除文件/分支、清空数据
- **难以回退**：force push、amend 已发布的 commit
- **影响他人**：推送代码、创建/关闭 PR、发送消息
- **影响共享状态**：修改基础设施、权限配置

### 17.4 用户权限模式

通过 Task 工具的 `mode` 参数控制子 Agent 的权限：

| 模式 | 说明 |
|------|------|
| `default` | 默认模式，需要用户确认 |
| `acceptEdits` | 自动接受文件编辑 |
| `dontAsk` | 不询问直接执行 |
| `bypassPermissions` | 绕过所有权限检查 |
| `plan` | Plan Mode，需要计划审批后才能编辑文件 |

---

## 18. 核心行为准则

### 18.1 编码原则

| 原则 | 说明 |
|------|------|
| 先读后改 | 读取理解现有代码后再提建议 |
| 最小变更 | 只做用户要求的修改，不添加额外功能 |
| 不过度工程 | 不为假设性需求设计，三行重复代码好过过早抽象 |
| 安全优先 | 避免命令注入、XSS、SQL 注入等 OWASP Top 10 漏洞 |
| 优先编辑 | 优先编辑现有文件，而非创建新文件 |
| 不加不需要的东西 | 不给未改动的代码加 docstring / 注释 / 类型注解 |
| 不要向后兼容 hack | 如确认无用的代码，直接删除，不留 `_unused`、注释等 |

### 18.2 沟通原则

| 原则 | 说明 |
|------|------|
| 简洁回复 | 回答简短直接 |
| 不用 emoji | 除非用户要求 |
| 代码引用格式 | `file_path:line_number` |
| 不估算时间 | 不预测任务耗时 |
| 遇阻不暴力 | 不反复重试失败操作，考虑替代方案或询问用户 |

---

## 19. 工具使用优先级

### 19.1 替代关系

| 操作 | 应使用的工具 | 不应使用的工具 |
|------|-------------|---------------|
| 读取文件 | `Read` | `cat`, `head`, `tail` |
| 编辑文件 | `Edit` | `sed`, `awk` |
| 创建文件 | `Write` | `echo >`, `cat <<EOF` |
| 搜索文件名 | `Glob` | `find`, `ls` |
| 搜索文件内容 | `Grep` | `grep`, `rg` |
| Git/系统命令 | `Bash` | - |
| 复杂多步探索 | `Task (Explore)` | 手动多次 Grep |

### 19.2 搜索策略

```
简单定向搜索（找特定文件/类/函数）
    → 直接用 Glob 或 Grep

广泛代码库探索（深度研究）
    → 用 Task (subagent_type: Explore)
    → 比直接 Grep 慢，但适合需要 3+ 次查询的场景

代码库理解 + 实现方案设计
    → 用 Task (subagent_type: Plan)
```

### 19.3 并行策略

- 独立的工具调用应并行发起（在同一条消息中）
- 有依赖关系的调用必须串行（等前一个完成后再调用下一个）
- Bash 中独立的命令用多个并行 Bash 调用；有依赖的用 `&&` 串联

---

## 20. Auto Memory 机制

### 20.1 什么是 Auto Memory

Claude Code 有一个**跨会话持久化的记忆目录**，路径为：
```
~/.claude/projects/{project-path}/memory/
```

其中 `MEMORY.md` 文件会**自动加载到每次对话的上下文中**（前 200 行），其他文件按需读取。

### 20.2 记忆管理规则

**应该保存的**：
- 多次交互中确认的稳定模式和约定
- 关键架构决策、重要文件路径、项目结构
- 用户的工作流偏好
- 反复出现的问题的解决方案

**不应该保存的**：
- 会话特定的上下文（当前任务细节、进行中的工作）
- 可能不完整的信息 — 写入前应先核实
- 与 CLAUDE.md 指令重复或矛盾的内容
- 读取单个文件后的推测性结论

**用户明确要求时**：
- 用户说 "记住这个" → 直接保存，不需等多次交互
- 用户说 "忘记这个" → 找到并删除相关记忆

### 20.3 记忆文件组织

```
memory/
├── MEMORY.md          # 主记忆文件（自动加载，保持简洁 <200 行）
├── debugging.md       # 调试相关笔记
├── patterns.md        # 代码模式和约定
└── ...                # 按主题语义组织，不按时间
```
