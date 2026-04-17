# 运行模型：Daemon（常驻进程）

CC/Codex 是 workspace 可切换的会话隔离应用——每次 `claude` 启动是一个全新会话。**Sage 不同：一个 Agent 固定一个 workspace，固定一个领域。** 对这样的 Agent，最自然的运行模型是 **Daemon**。

## Daemon 架构

```
sage start feishu     → 启动 feishu daemon（VM 启动，Memory 注入，Agent 待命）
sage connect feishu   → 接入 TUI，和 Agent 对话（可多次 connect/disconnect）
sage disconnect       → 断开 TUI（Agent 继续运行，等待下次连接）
sage stop feishu      → 关闭 daemon，VM 销毁，资源释放

sage send feishu "查下明天日程"   → 非交互式，发一条消息，等结果返回
sage status                       → 列出所有运行中的 Agent daemon
```

**Daemon Loop = Agent ReAct 三层循环之上的第 4 层循环：**

```
第 4 层 DaemonLoop（新增，代码层，不可 YAML 配置）
  STARTUP: 加载 AgentConfig、启动 VM、注入 Memory、加载 Skills、注册 Hooks
  loop {
    IDLE:
      ┌─ PostProcessing hook：检查是否有未处理的 session 记录
      │  → 有：进入 WIKI_MAINTENANCE（见下方）
      │  → 无：rx.recv().await  ← 挂起等待用户消息，让出 tokio 调度权
      └─
    PROCESSING (SessionType::UserDriven):
      fire UserPromptSubmit hook
      第 3 层 OUTER LOOP（follow-up 队列）
        第 2 层 INNER LOOP（steering + tool calls）
          第 1 层 INNERMOST（LLM 流式调用）
          fire PreToolUse hook → tool 执行 → fire PostToolUse hook
      fire Stop hook（exit 2 = 要求继续）
      归档对话记录到 raw/sessions/（标记 unprocessed）
    WIKI_MAINTENANCE (SessionType::WikiMaintenance):
      注入 wiki-ingest Skill 到 Agent 上下文
      向 Agent 发送自动消息：整理未处理的 session 记录
      Agent 进入 Agent Loop（使用标准工具，按 wiki Skill 指令操作）
      完成后标记 sessions 为 processed
      ※ 此期间用户新消息进入优先级队列，不中断维护
      ※ 维护完成后立即处理队列中的用户消息
    回到 IDLE
  }
  SHUTDOWN: fire SessionEnd hook → VM 销毁
```

**所有 Hook / Skill / Wiki / Harness 机制均在 DaemonLoop 层执行，AgentLoop 本身不感知。**

```
Caster 进程（tokio runtime）
├── DaemonLoop A: feishu   [async task]
├── DaemonLoop B: github   [async task]
├── DaemonLoop C: research [async task]
├── VM Pool（LRU 调度，容量 = M 个热 VM）
└── Unix socket server（接受 TUI / sage send / Rune 连接）
```

## VM Pool：LRU 调度（资源最大化利用）

Daemon Loop 是轻量 async task（无限多），VM 是重量级资源（有限 M 个）。VM Pool 用 LRU 算法调度两者之间的资源分配：

**Daemon Loop 的四种状态：**

| 状态 | VM | 有 Agent | 有活跃用户 | 说明 |
|---|---|---|---|---|
| `IDLE_HOT` | 运行中 | 是 | 否 | 刚处理完，VM 保温，快速响应 |
| `IDLE_COLD` | 无（已驱逐） | 是 | 否 | 长期空闲，VM 被回收，下次 cold start |
| `PROCESSING` | 运行中 | 是 | 是 | 正在执行 Agent Loop |
| `BLOCKED` | 等待分配 | 是 | 是（有消息） | 等 VM Pool 有空槽位 |

**LRU 驱逐策略：**

```
Daemon Loop 收到消息 → 向 VM Pool 申请槽位
  ├── 该 Agent 已有热 VM → 直接复用（fast path，毫秒级）
  ├── Pool 未满 → 启动新 VM（cold start，~1-2s）
  └── Pool 已满 → 驱逐最久未使用的 IDLE_HOT Agent
      → 终止其 VM（对话历史在 HOST 侧不受影响）
      → 为当前 Agent 启动新 VM

Daemon Loop 回到 IDLE → 启动 idle_vm_timeout 倒计时
  └── 超时无新消息 → 主动释放 VM 槽位（IDLE_HOT → IDLE_COLD）
```

**为什么不需要 VM Snapshot：**

对话历史存在 HOST 侧的 DaemonLoop 内存中，VM 只是工具执行沙箱。IDLE 状态的 Agent 没有 in-progress 的工具执行，VM 被终止不会丢失任何对话状态。下次启动一个干净的 VM，注入现有对话历史即可继续。

## 双层调度：横向 × 纵向

```
Rune Runtime（横向扩缩容）
  ├── 监控各 Caster 的 BLOCKED 队列压力
  ├── 压力高 → spawn 新 Caster 进程（新 M 个 VM 槽）
  └── 压力低 → 终止空闲 Caster 进程

每个 Caster（纵向 VM Pool 管理）
  ├── N 个 DaemonLoop（async tasks，数量 = 该 Caster 服务的 Agent 数）
  ├── M 个 VM 热槽（capacity 由 Caster 启动参数决定，默认 3-5）
  └── LRU 驱逐：保证 M 个槽给最活跃的 Agent

两层职责：
  Rune Runtime = 进程级调度（Caster 数量）
  VM Pool      = 槽位级调度（单 Caster 内 VM 分配）
```

## 为什么 Daemon 解决了所有会话问题

- **Agent 不需要知道"什么时候结束"** — 它不结束，只是等待
- **没有 session 边界问题** — 对话线是连续的，connect/disconnect 只是接入/断开
- **知识蒸馏时机自然出现** — PreCompact hook：compact 前先让 Agent 把值得记住的写入 Memory.md，再压缩对话历史

```
对话线增长 → context 接近上限 → PreCompact hook 触发
  → Hook 提示 Agent：有什么值得永久记忆的？更新 MEMORY.md
  → compact（压缩/摘要旧对话）
  → 继续（新 context = 压缩后历史 + Memory 仍然在 system prompt 里）
```

## 与 CC/Codex 的本质差异

| | CC / Codex | Sage |
|---|---|---|
| 运行模型 | 会话式（启动/退出） | Daemon（常驻） |
| workspace | 可切换（不同项目） | 固定（每个 Agent 一个） |
| 对话历史 | 会话内，退出清空 | 连续积累，compaction 管理 |
| 知识持久化 | CLAUDE.md（用户手动维护） | MEMORY.md（Agent 主动更新） |
| 知识更新时机 | 用户手动 | Agent 写 + PreCompact hook 蒸馏 |

## 知识与历史的四层分层

```
对话历史（messages）：连续增长，compaction 定期压缩
  └─ 作用：Agent 的"工作记忆"，当前任务上下文
  └─ 生命周期：daemon 存活期间，compact 时压缩但不清空
  └─ 存储：HOST 侧 DaemonLoop 内存

Raw Sessions（raw/sessions/）：对话历史的持久化归档
  └─ 作用：Wiki 维护的输入源——IDLE 时 Agent 从 raw sessions 中提炼知识
  └─ 生命周期：永久（ingest 后标记 processed，不删除）
  └─ 存储：workspace/raw/sessions/*.jsonl

Memory（memory/）：操作性短期记忆
  └─ 作用：用户偏好、决策上下文、当前项目状态
  └─ 生命周期：中期（会被覆盖/清理，跨 compact 存活）
  └─ 存储：workspace/memory/（MEMORY.md 注入 system prompt）

Wiki（wiki/）：结构化领域知识
  └─ 作用：概念定义、模式识别、陷阱记录、跨主题综合分析
  └─ 生命周期：长期（知识复利积累，越用越厚）
  └─ 存储：workspace/wiki/（通过 wiki_search 按需检索，不注入 system prompt）

Craft（craft/）：程序性记忆
  └─ 作用：固化的操作流程，经过 token 效率评估的 SOP
  └─ 生命周期：长期（评分高的稳定保留，评分低的被迭代优化）
  └─ 存储：workspace/craft/（frontmatter 注入 system prompt，正文按需加载）

知识流动方向：
  对话历史 → (compact 前) → Memory（碎片事实）
  对话历史 → (IDLE 时) → raw/sessions/ → (WikiMaintenance) → Wiki（结构化知识）
  反复出现的任务模式 → (CraftEvaluation) → Craft（操作流程）
  Wiki + Craft → (长期积累) → AGENT.md Schema（顶层认知框架）
```

## Channel 架构（v0.9.x）

Channel Adapter 使 Agent 能接入 TUI、飞书、Slack 等平台，Agent 不感知投递目标。

**核心设计原则**：
- **一个 Session 绑定一个 Channel**：路由逻辑由 Channel Adapter 处理，Agent 不感知投递目标
- **CHANNEL_HINTS 注入 system prompt**：LLM 知道自己在哪个平台上，自动适配输出格式
- **AgentEvent 三级可见性过滤**：`Developer`（TUI 专属）/ `User`（所有 Channel）/ `Internal`（日志系统）

```rust
trait ChannelAdapter: Send + Sync {
    fn channel_hints(&self) -> &str;           // 注入 system prompt 的平台描述
    fn visibility_filter(&self) -> Visibility; // 事件过滤规则
    async fn send(&self, event: AgentEvent) -> Result<()>;
}
```

**CHANNEL_HINTS 完整流（以飞书为例）**：

```
飞书群消息 → FeishuChannel webhook
  → 查找/创建 feishu DaemonLoop Session
  → SessionStart hook:
      SystemPromptBuilder += section("channel_hints", feishu_hints)
  → 消息入队，DaemonLoop PROCESSING
  → LLM 根据 channel_hints 格式化输出（已知在飞书上）
  → AgentEvent(User, TextDelta) → FeishuChannel.send()
  → 飞书卡片消息发送到群
  → Agent 回到 IDLE_HOT，等待下一条消息
```

## 触发系统（v0.9.x C7）

触发系统是 Sage **应用级**基础设施，不属于单个 Agent 的配置。触发器只做一件事：构造消息，路由到目标 Agent 的 DaemonLoop 消息队列。

```yaml
# ~/.sage/triggers.yaml（应用级，独立于 agent.yaml）
triggers:
  - name: morning-brief
    type: cron
    schedule: "0 9 * * 1-5"       # 工作日早 9 点
    route_to: feishu-agent
    message: "生成今日飞书日程摘要，发送到飞书"

  - name: feishu-inbound
    type: feishu_event
    event: im.message.receive_v1   # 飞书消息抵达（与 C4 共享 webhook 入口）
    route_to: feishu-agent
    # message 由 Channel Adapter 从事件 payload 中提取，无需手动配置
```

触发类型：

| 类型 | 信号 | 适用场景 |
|---|---|---|
| `cron` | 时间表达式（cron syntax） | 定时任务、日报、周报 |
| `feishu_event` | 飞书 Open Platform 事件 | 飞书消息触发（与 FeishuChannel 共享 webhook） |
| `file_watch` | inotify / FSEvents 文件变化 | 监听 inbox/ 目录有新文件就处理 |
