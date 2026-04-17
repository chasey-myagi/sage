# 领域专家架构：四层知识体系

一个"领域专家 Agent"的可信赖性，依赖四层各自独立、相互协作的知识体系：

| 层 | 机制 | 解决的问题 | 更新频率 | 由谁维护 | 存储位置 |
|---|---|---|---|---|---|
| **Hook** | 生命周期钩子（代码层 Harness） | 机械性保证：质量门控、安全拦截、确定性行为 | 低 | 开发者 / 平台 | agent.yaml / hooks.yaml |
| **Skill** | Markdown 领域知识模块（全局只读） | 领域认知：标准 SOP、最佳实践、通用任务流程 | 低 | 开发者 / 领域专家 | ~/.sage/skills/ |
| **Craft** | Agent 自管理的可复用产物（任意类型） | 程序性记忆：SOP、脚本、模板、素材——Agent 从实践中沉淀的工具箱 | 中 | Agent 自管理 | workspace/craft/ |
| **Wiki** | LLM 维护的结构化领域知识库 | 领域知识沉淀：概念、模式、陷阱、跨主题综合分析 | 中-高 | Agent（Daemon IDLE 时自动维护） | workspace/wiki/ |
| **Memory** | 工作空间内的操作性短期记忆 | 用户个性化：偏好、决策上下文、当前项目状态 | 高 | Agent + 用户交互 | workspace/memory/ |

## 为什么四层缺一不可

- **只有 Hook，没有 Skill/Wiki**：知道该 stop，但不知道该怎么做。质量门在那，但 Agent 的领域能力是空白的。
- **只有 Skill，没有 Wiki**：知道操作步骤，但不理解领域全貌。有 SOP 但没有 know-how——会按步骤做，但遇到意外无法举一反三。
- **只有 Wiki，没有 Skill**：理解领域知识，但没有固化的操作流程。每次执行同类任务都要重新推理，token 浪费严重。
- **只有 Memory，没有 Wiki**：记住了用户说的话，但没有结构化提炼。信息碎片化——"上次你说过 X"但不理解 X 背后的规律。

四层叠加：**Hook 保证它是可控的，Skill 保证它是高效的，Wiki 保证它是有知识的，Memory 保证它是懂我的。**

## Karpathy LLM-Wiki 模式

Wiki 层的设计受 Karpathy 的 llm-wiki 模式启发。核心思想：**知识需要分层加工，raw sources → wiki pages → schema**，知识在加工过程中产生复利效应。

```
workspace/
├── SCHEMA.md             ← Wiki 锚点：wiki 根路径、frontmatter 规范、cross-ref 约定
├── AGENT.md              ← Schema 层（Karpathy 第三层，注入 system prompt）
├── raw/                  ← 第一层：不可变原始记录（Karpathy "raw sources"）
│   └── sessions/         #   每次对话的完整记录，ingest 后标记 processed
│       ├── 2026-04-14-001.jsonl
│       └── 2026-04-14-002.jsonl
├── wiki/                 ← 第二层：LLM 维护的结构化知识（Karpathy "wiki"）
│   ├── index.md          #   所有页面的目录索引（按类别分组）
│   ├── log.md            #   追加式维护日志（append-only）
│   ├── overview.md       #   全局综合理解（随知识积累不断演化）
│   └── pages/            #   所有 wiki 页面，flat 结构，slug 命名
│       ├── feishu-calendar-api.md
│       ├── batch-event-pattern.md
│       └── timezone-pitfall.md
├── assets/               ← 附件（图片、PDF 等）
├── memory/               ← 操作性短期记忆（Episodic → Semantic 的中间层）
│   ├── MEMORY.md         #   索引文件（注入 system prompt）
│   ├── user.md           #   用户画像
│   ├── context-*.md      #   当前项目上下文
│   └── decisions-*.md    #   关键决策记录
├── craft/                ← Agent 自管理的可复用产物（Procedural Memory）
│   ├── <craft-name>/     #   SOP 类：CRAFT.md + metrics.json
│   ├── <script>.py       #   脚本类：可执行脚本
│   └── <template>.md     #   模板类：输出模板
└── metrics/              ← 任务度量记录（TaskRecord）
    ├── <task_id>.json    #   每次 UserDriven 任务一条，ULID 命名
    └── summary.json      #   最近 N 条任务的滚动聚合
```

**Wiki 页面 frontmatter 规范**（由 SCHEMA.md 定义，wiki Skill 强制遵守）：

```markdown
---
title: 飞书日历 API 批量操作
tags: [api, calendar, batch]
sources: [session-2026-04-14-001]
updated: 2026-04-14
---
```

所有页面 flat 存放在 `wiki/pages/`，slug 格式（小写、连字符分隔、无特殊字符）。
使用 `[[slug]]` 语法交叉引用，如 `[[feishu-calendar-api]]` → `wiki/pages/feishu-calendar-api.md`。

## Wiki vs Memory vs Skill vs Craft 的边界

| 维度 | Memory | Wiki | Skill（全局） | Craft（workspace） |
|------|--------|------|--------------|-------------------|
| **内容性质** | 事实碎片（"用户偏好 X"） | 结构化知识（"X 的原理是..."） | 标准 SOP、API 模板（Markdown） | SOP / 脚本 / 模板 / 素材（任意类型） |
| **更新时机** | 对话中随时 | Daemon IDLE 时批量维护 | 开发者发版 | 任务成功后 Agent 自评估创建 |
| **生命周期** | 短-中期（会被覆盖/清理） | 长期（知识复利积累） | 长期（只读，随平台更新） | 长期（token 效率越高越稳定） |
| **注入方式** | system prompt 直接注入 | 按需检索（读 index.md → 读具体页面） | /skill-name 激活时注入 | craft_manage 工具调用时加载 |
| **写入方式** | Agent 直接写文件 | Agent 使用标准工具，由 Wiki Skill 约束行为模式 | 只读（不可写） | Agent 通过 craft_manage 工具创建/编辑 |

## 知识晋升路径

四层知识体系之间有明确的晋升路径——从即时观察到稳定的领域知识，每层更压缩、更长寿：

```
对话历史（Working Memory）
  ↓ compact 前 → 碎片事实写入 memory/
Episodic（raw/sessions/）：完整会话归档，不可变
  ↓ Daemon IDLE 时 wiki-ingest
Semantic（wiki/pages/）：从任务执行中提炼的领域规律
  ↓ 反复出现的模式 → 固化为 Craft
Procedural（craft/）：SOP、脚本、模板、素材，token 效率评分驱动进化
```

`TaskRecord`（`metrics/`）是评测传感器，与晋升路径并行存在：每次 UserDriven 会话结束时自动写入，记录 token 消耗、轮次、工具调用、成功与否。数据积累后，将驱动 Skill efficiency_score 更新和 Agent 配置自优化。

## Wiki Skill 包：chasey-myagi/llm-wiki

Wiki 不用自定义 Tool，而是通过 **Skill 约束 Agent 的行为**——Agent 用标准文件工具（Read / Write / Glob / Grep），Skill 指令确保它按 llm-wiki 规范操作。

**参考来源**：[kfchou/wiki-skills](https://github.com/kfchou/wiki-skills)——提供了 5 个 wiki 技能的结构和交互模式。

**Sage 的关键区别**：kfchou/wiki-skills 设计为研究型知识库（ingest 外部论文/URL），Sage 的 wiki 是**任务执行型知识库**——source 是 Agent 自己的 session records，内容是"做任务中学到的领域规律"，而不是"这篇文章说了什么"。

**`chasey-myagi/llm-wiki` 包含 4 个 Skills**（去掉 wiki-init，Agent 首次启动时由 Sage 自动创建 wiki 结构）：

| Skill | 适配后的功能 | 何时触发 |
|-------|------------|---------|
| `wiki-ingest` | 读取 raw session → 提炼领域规律 → 写入 wiki 页面 | Daemon IDLE 时自动 |
| `wiki-query` | 查阅 wiki 中已有的领域知识，带 `[[slug]]` 引用 | Agent 执行任务需要回忆规律时 |
| `wiki-lint` | 健康检查：断链、孤立页、过期页 | 每 5-10 次 ingest 后 |
| `wiki-update` | 修订已有页面（知识变更时），展示 diff 后写入 | 发现已有知识需要纠正时 |

**Skill 的核心约束**（prompt 引导而非 Tool API 强制）：

- **SCHEMA.md 是锚点**：wiki Skill 启动前先读 SCHEMA.md，获取 wiki 根路径、frontmatter 规范、`[[slug]]` cross-reference 约定
- **页面结构**：每个页面必须有 YAML frontmatter（title、tags、sources、updated）
- **索引同步**：写入/修改页面后必须更新 index.md
- **日志追加**：所有操作追加 log.md（append-only）
- **双向链接**：新页面创建后扫描已有页面，添加 backlink（wiki 价值来自交叉引用密度）

**wiki-ingest 的 Sage 适配**（与 kfchou 原版的关键差异）：

kfchou 的 source 是外部文档（"这篇论文讲了什么"），Sage 的 source 是 session records（"做这次任务我学到了什么"）。提炼角度不同：

```
kfchou 写法：  "本文提出了 Transformer 架构，核心贡献是..."
Sage 写法：    "飞书日历 API 批量写入的坑：超过 50 条时必须分批，否则 rate limit 报错"
              "正确模式：每批 ≤ 20 条，批间 sleep 200ms，重试 3 次"
```

Sage 的 wiki 页面类型：
- **pitfall**：踩过的坑（具体报错 + 正确做法）
- **pattern**：反复有效的操作模式（步骤 + 适用场景）
- **api-ref**：领域 API 的关键行为（不是文档抄写，是实践发现）
- **decision**：做过的重要决策 + 理由（避免同一问题反复权衡）

**为什么 Skill 而不是 Tool**：

| 维度 | 自定义 Tool | Skill |
|------|------------|-------|
| 实现成本 | Rust 侧注册 5 个 Tool，重新编译 | 纯 Markdown 文件，零代码 |
| 修改灵活性 | 改行为需发版 | 改 Skill 文件即生效 |
| 可进化性 | 不变 | Agent 可在 workspace/craft/ 创建改进版 |

**自动安装机制**：

```
Agent 初始化时（DaemonLoop STARTUP）：
  1. 检查 .agent/skills/ 是否已有 llm-wiki Skills
  2. 没有 → 从 chasey-myagi/llm-wiki 安装到 .agent/skills/（只读）
  3. 同时初始化 workspace/wiki/ 目录 + SCHEMA.md + index.md + log.md + overview.md
```

## Wiki 维护：Daemon Loop 集成

Wiki 维护发生在 **Daemon Loop 的 IDLE 阶段**，不在 Agent Loop 内——避免破坏 ReAct 框架。

```
DaemonLoop IDLE 状态
  → PostProcessing hook 检查：是否有未处理的 session 记录？
  → 如果有：
      1. 标记当前会话类型为 WikiMaintenance（避免循环触发）
      2. 注入 wiki-ingest Skill 的完整内容到 Agent 上下文
      3. 向 Agent 发送自动消息："请处理以下未整理的对话记录，按照 wiki-ingest 规范更新 wiki"
      4. Agent 进入 Agent Loop，使用标准工具（Read/Write/Glob）按 Skill 指令操作 wiki/
      5. 完成后标记 sessions 为 processed
      6. 回到 IDLE
  → WikiMaintenance 会话期间：
      - 用户新消息进入 message queue（优先级队列）
      - 不中断 wiki 维护（中断 = 浪费 token）
      - wiki 维护完成后，立即处理队列中的用户消息
```

**会话类型标记（防止无限循环）**：

| SessionType | 说明 | 触发 wiki 维护？ |
|---|---|---|
| `UserDriven` | 正常用户对话 | 结束后 → 是 |
| `WikiMaintenance` | wiki 整理会话 | 否（避免循环） |
| `CraftEvaluation` | skill 评估/创建 | 否 |
