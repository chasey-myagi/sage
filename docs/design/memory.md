# 分层记忆设计（GA L0–L4）

参考：GenericAgent（lsdefine/GenericAgent）的 L0–L4 + skill 树。
[研究报告对照](file:///Users/chasey/Dev/cc/research-reports/oh-my-pi-vs-sage/index.html)

## 为什么这样分

GA 用 3K 行核心代码 + ~30K 上下文做到了别人 200K–1M 才做到的事。关键在于**记忆不在 prompt 里，在 workspace 文件系统里**，按需 read。

Sage 抄这套姿态。每个领域 Agent 的 workspace 长这样：

```
~/Dev/cc/<domain>/
├── AGENT.md              ← L0：元规则（行为约束 / 写入规范 / 风格）
├── MEMORY.md             ← L1：索引（指向 facts/、craft/、sessions/）
├── facts/                ← L2：全局事实
│   ├── tables.md
│   └── contacts.md
├── craft/                ← L3：任务 SOP（可复用脚本和流程）
│   ├── add-company.sh
│   ├── list-upcoming.sh
│   └── interview-flow.md
├── sessions/             ← L4：会话归档（任务完成后压缩）
└── .sage/
    ├── settings.json     ← 工具白名单
    └── working.md        ← 短期工作 notepad（每轮自动注入，最多 200 token）
```

## 自动注入策略

启动 sage 时，按层级把这些**自动塞进 system_prompt**：

| 层 | 注入方式 | 大小预算 |
|---|---|---|
| L0 `AGENT.md` | 总是注入到 system_prompt 末尾 | ≤ 2KB |
| L1 `MEMORY.md` | 总是注入（索引而已） | ≤ 1KB |
| L2 `facts/` | **不自动注入**——agent 通过 `read` 工具按需读 | n/a |
| L3 `craft/` | 文件名 + 一行描述自动注入；内容按需 read | ≤ 500 字节/条 |
| L4 `sessions/` | 不注入——只有 agent 主动检索时才读 | n/a |

启动时 prompt cache 命中率会高（L0+L1 不变），中间过程动态 read L2/L3，结束时新经验固化进 L3 或归档进 L4。

## working.md：每轮 200 token 小本本

GA 的 `update_working_checkpoint` 工具——每轮调用都把当前进度/约束写进一个独立文件，下一轮自动注入。防止"50 轮之后 agent 忘了一开始定的口径"。

实现要求：
- 单文件 `working.md`，agent 用 `update_working` 工具覆盖式写入
- 每轮 agent_loop 在调 LLM 前把 working.md 读出拼到 prompt 末尾
- 上限 200 token（超出截断尾部）
- 任务结束时归档到 `sessions/<task_id>.md`

## 落地优先级

v0.5 实现顺序：
1. L0 `AGENT.md` 自动注入（最简单，最早收益）
2. L3 `craft/` 索引扫描 + 文件名/描述注入
3. L1 `MEMORY.md` 索引
4. working.md + `update_working` 工具
5. L4 session 归档（可推迟）

## 不抄的东西

- ❌ 不做 wiki（SCHEMA.md / wiki/index.md / wiki/log.md / wiki/pages/）——过度设计，先用 markdown 文件名做索引
- ❌ 不做 craft 评分系统（metrics.json / summary.json）——没数据，先沉淀
- ❌ 不做"百万 skill 库"——你不是 GA
