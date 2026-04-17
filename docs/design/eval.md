# 评测系统：TaskRecord

每次任务的行为快照。**四层知识体系的传感器**——没有它，Skill 的 efficiency_score 是空的，"基于数据优化 Agent"无从谈起。

## 数据来源（零侵入）

`MetricsCollector` 订阅已有的 AgentEvent 流，无需修改 agent_loop：

| 事件 | 采集内容 |
|---|---|
| `TurnEnd.message.usage` | 每轮 input / output / cache token 消耗（逐轮累加） |
| `ToolExecutionEnd` | 工具调用次数、失败次数 |
| `CompactionEnd` | 压缩次数 |
| `AgentEnd` / `RunError` | 任务成功 / 失败 + 原因 |

## TaskRecord 结构

写入 `workspace/metrics/<task_id>.json`，ULID 命名：

```rust
pub struct TaskRecord {
    pub task_id: String,           // ULID
    pub agent_name: String,
    pub model: String,
    pub config_hash: String,       // sha256(agent.yaml) → 追踪哪套配置

    pub started_at: u64,
    pub ended_at: u64,
    pub duration_ms: u64,

    // token 消耗
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,

    // 行为计数
    pub turn_count: u32,
    pub tool_call_count: u32,
    pub tool_error_count: u32,
    pub compaction_count: u32,

    // 结果
    pub success: bool,
    pub failure_reason: Option<String>,

    // 知识层（Phase 2+）
    pub crafts_active: Vec<String>,  // 本次注入了哪些 Craft（Phase 2+）
}
```

## 存储策略

- `workspace/metrics/<task_id>.json`：每次任务一条，仅 UserDriven 会话写入（WikiMaintenance / CraftEvaluation 不写）
- `workspace/metrics/summary.json`：最近 50 条任务的滚动聚合（avg/p95 tokens、成功率、常用工具统计）

## 实现任务（v0.7.0 M3）

- [ ] `MetricsCollector` 订阅 AgentEvent 流（零侵入 agent_loop），累积每轮 `TurnEnd.message.usage`、工具调用/错误次数、压缩次数
- [ ] UserDriven 会话结束时写 `workspace/metrics/<task_id>.json`（ULID 命名）
- [ ] 定期更新 `workspace/metrics/summary.json`（最近 50 条任务的滚动聚合）
- [ ] WikiMaintenance / CraftEvaluation 会话不写 TaskRecord（避免污染用户任务数据）

## 未来用途（数据积累后启用）

积累足够的 TaskRecord 后，可以回答：

| 问题 | 数据支撑 |
|---|---|
| 这个 Skill 有没有用？ | 有/无该 Skill 时 mean_tokens 的对比 |
| 这套 model 配置性价比如何？ | (success_rate / mean_cost) by model |
| 知识积累后任务变便宜了吗？ | 第 N 次 vs 第 1 次同类任务的 token 差 |

本质是：**在没有梯度的世界里，用经验数据做提示蒸馏。** 不是反向传播，而是观察→沉淀→提炼→优化的闭环。
