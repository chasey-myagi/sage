# Harness Engineering Design

> **定位**：Harness 是 Sage 的 Agent 行为质量保证系统——定义"任务完成"的标准，并通过可组合的 Eval 脚本强制执行。
>
> 参考文档：[hook-craft.md](hook-craft.md)（Hook 机制）、[eval.md](eval.md)（TaskRecord）、[research-cc-hooks-skills.md](../research-cc-hooks-skills.md)

---

## 一、为什么需要 Harness

LLM 驱动的 Agent 有一个本质问题：**它在什么时候真的完成了任务？**

`stop_reason: end_turn` 只说明 LLM 认为它完成了，不代表任务结果是正确的。没有外部验证机制，Agent 可以：
- 产生幻觉（假装执行了但没有）
- 完成了错误的任务
- 在复杂任务中提前放弃

Harness 的底层逻辑：**把"任务完成"的判断权从 LLM 手里拿回来，交给确定性代码。**

| 无 Harness | 有 Harness |
|-----------|-----------|
| Agent 自报完成 | 外部脚本验证完成 |
| 结果正确性未知 | 标准可量化 |
| 调试靠肉眼看 | 失败有可观测原因 |
| CI 无法自动测试 Agent | `sage test` 跑 Suite |

---

## 二、核心机制：Stop Hook as Eval Entry

Harness 的核心挂载点是 **`Stop` hook**。这是 Sage 生命周期中 Agent 结束每轮响应前的最后一个 checkpoint。

```
LLM 完成本轮输出（stop_reason: end_turn）
  │
  ▼
Stop Hook Runner（执行所有注册的 eval 脚本）
  │
  ├─ exit 0  → 全部通过 → Agent 正式结束，结果返回给调用方
  ├─ exit 2  → 验证失败 → stderr 注入 Agent 上下文，Agent 继续（再来一轮）
  └─ exit 1  → Harness 系统错误 → 展示给用户，Agent 结束
```

**exit 2 的魔力**：Agent 不会知道被"拒绝"了——它只会看到一条新消息告诉它哪里不对，然后自己重新尝试。Harness 对 Agent 是透明的，但牢牢控制着"完成"的定义权。

```
[第 1 轮] Agent 写文件 → LLM end_turn → Stop hook: exit 2（"文件内容格式不对"）
                                                          ↓
[第 2 轮] Agent 收到反馈，修正格式 → LLM end_turn → Stop hook: exit 0 ✓
                                                          ↓
                                                     任务完成，返回结果
```

---

## 三、Eval 脚本契约（command hook）

Harness eval 脚本是标准 `command` 类型的 Stop hook，遵循以下 I/O 契约：

### 3.1 输入（stdin JSON）

```json
{
  "stop_reason": "end_turn",
  "session_id": "01HZ...",
  "task_id": "01HZ...",
  "turn_count": 3,
  "agent_name": "feishu",
  "model": "claude-sonnet-4-20250514"
}
```

> 注：workspace 内容通过文件系统直接访问（eval 脚本和 Agent 挂载同一 workspace），不通过 stdin 传递。

### 3.2 输出

**exit 0（通过）**：
```json
// stdout（可选，写入 TaskRecord 的 harness_result 字段）
{
  "passed": true,
  "score": 1.0,
  "criteria": [
    { "name": "file_exists", "passed": true },
    { "name": "format_valid", "passed": true }
  ]
}
```

**exit 2（失败，要求 Agent 继续）**：
```
// stderr（注入 Agent 上下文，Agent 会看到这段话）
验证失败：/workspace/output.json 存在，但缺少必要字段 "event_id"。
请检查飞书 API 响应，确保写入完整的事件对象。
```

**exit 1（Harness 系统错误）**：
```
// stderr（展示给用户，不注入 Agent）
eval 脚本依赖的 jq 工具不存在，请安装后重试。
```

### 3.3 Eval 脚本示例

**示例 1：检查文件存在并格式正确**（Shell）

```bash
#!/usr/bin/env bash
set -euo pipefail

WORKSPACE="/workspace"
OUTPUT="$WORKSPACE/output.json"

# 检查文件存在
if [ ! -f "$OUTPUT" ]; then
  echo "验证失败：未生成 $OUTPUT，请确保任务完成后写入结果文件。" >&2
  exit 2
fi

# 检查必要字段
if ! jq -e '.event_id and .summary and .start_time' "$OUTPUT" > /dev/null 2>&1; then
  MISSING=$(jq -r 'to_entries | map(select(.value == null) | .key) | join(", ")' "$OUTPUT")
  echo "验证失败：output.json 缺少字段: $MISSING" >&2
  exit 2
fi

# 输出详细结果
jq -n '{passed: true, score: 1.0, criteria: [{name: "file_exists", passed: true}, {name: "schema_valid", passed: true}]}'
exit 0
```

**示例 2：调用 API 验证副作用**（Python）

```python
#!/usr/bin/env python3
import json, sys, os
import lark_oapi

def check():
    # 从 workspace 读取 Agent 记录的操作日志
    log_path = "/workspace/harness-log.json"
    if not os.path.exists(log_path):
        print("验证失败：未找到操作日志，Agent 可能未执行飞书 API 调用。", file=sys.stderr)
        sys.exit(2)

    with open(log_path) as f:
        ops = json.load(f)

    # 检查期望操作是否发生
    calendar_updates = [op for op in ops if op.get("type") == "calendar.update"]
    if not calendar_updates:
        print("验证失败：日志中无日历更新操作记录。", file=sys.stderr)
        sys.exit(2)

    # 可选：实际调用飞书 API 验证日历变更
    # client = lark_oapi.Client.builder().app_id(os.environ["FEISHU_APP_ID"])...

    result = {
        "passed": True,
        "score": 1.0,
        "criteria": [
            {"name": "log_exists", "passed": True},
            {"name": "calendar_updated", "passed": True, "count": len(calendar_updates)}
        ]
    }
    print(json.dumps(result))
    sys.exit(0)

check()
```

---

## 四、HarnessConfig：配置格式

### 4.1 Agent 级 Harness（agent.yaml 内嵌）

```yaml
# ~/.sage/agents/feishu/agent.yaml
hooks:
  Stop:
    - hooks:
        - type: command
          command: "bash ~/.sage/agents/feishu/harness/check-output.sh"
          timeout: 30
```

这是最简单的方式：给 Agent 挂一个 Stop eval 脚本，所有会话都验证。

### 4.2 独立 Harness Suite（用于 sage test）

```yaml
# tests/feishu-calendar.yaml
name: feishu-calendar-suite
description: "飞书日历操作能力测试"

# 共享配置（可被 case 覆盖）
defaults:
  agent: feishu
  sandbox: { mode: none }
  max_turns: 15
  timeout_secs: 120

cases:
  - name: create-single-event
    task: "在飞书日历创建明天 10:00 的团队会议，时长 1 小时，标题：项目同步"
    criteria:
      - type: command
        command: "bash tests/criteria/check-calendar-event.sh"
        description: "验证日历事件已创建"

  - name: batch-reschedule
    task: "把本周所有标题含'面试'的日历事件推迟一天"
    max_turns: 20
    criteria:
      - type: command
        command: "bash tests/criteria/check-batch-reschedule.sh"
        description: "验证批量重排完成"
      - type: command
        command: "bash tests/criteria/check-no-data-loss.sh"
        description: "验证原数据未损坏"

  - name: timezone-handling
    task: "创建一个北京时间 9:00 的会议，邀请旧金山同事（太平洋时间）"
    criteria:
      - type: command
        command: "python3 tests/criteria/check-timezone.py"
        description: "验证时区处理正确"
```

### 4.3 Criteria 类型

| 类型 | 配置 | 适用场景 |
|------|------|---------|
| `command` | `command: "..."` | 80% — 文件检查、API 验证、格式校验 |
| `prompt` | `prompt: "评估..."` | 语义判断（输出是否恰当、格式是否友好） |
| `agent` | `agent: "..."` | 复杂验证（需要工具调用的核对） |

多个 criteria **顺序执行，全部通过才算通过**。第一个失败即停止，失败原因注入 Agent。

---

## 五、sage test 命令

```bash
# 运行完整 Suite
sage test --suite tests/feishu-calendar.yaml

# 运行单个 case
sage test --suite tests/feishu-calendar.yaml --case create-single-event

# 指定 Agent（覆盖 suite 中的 defaults.agent）
sage test --suite tests/feishu-calendar.yaml --agent feishu-dev

# 并发运行（default: 1，串行）
sage test --suite tests/feishu-calendar.yaml --parallel 3

# CI 模式（输出 JUnit XML）
sage test --suite tests/feishu-calendar.yaml --reporter junit --output test-results.xml

# JSON 输出（便于脚本消费）
sage test --suite tests/feishu-calendar.yaml --reporter json
```

### 5.1 Terminal 输出（默认）

```
sage test — feishu-calendar-suite

  ✓  create-single-event       2.3s   turn:4   tokens:1847
  ✓  batch-reschedule          8.1s   turn:9   tokens:4203
  ✗  timezone-handling         5.2s   turn:12  tokens:3891
     └─ check-timezone.py 验证失败：
        期望 UTC 时间 01:00，实际写入 09:00（未做时区转换）

3 cases | 2 passed | 1 failed | 15.6s total
```

### 5.2 JSON 输出（--reporter json）

```json
{
  "suite": "feishu-calendar-suite",
  "passed": 2,
  "failed": 1,
  "total": 3,
  "duration_ms": 15600,
  "cases": [
    {
      "name": "create-single-event",
      "status": "passed",
      "turn_count": 4,
      "tokens": { "input": 1234, "output": 613, "cache_read": 0 },
      "criteria_results": [
        { "name": "check-calendar-event.sh", "passed": true }
      ],
      "duration_ms": 2300
    },
    {
      "name": "timezone-handling",
      "status": "failed",
      "turn_count": 12,
      "failed_criterion": "check-timezone.py",
      "failure_reason": "期望 UTC 时间 01:00，实际写入 09:00（未做时区转换）",
      "tokens": { "input": 2891, "output": 1000, "cache_read": 2300 }
    }
  ]
}
```

### 5.3 JUnit XML（--reporter junit，用于 GitHub Actions）

```xml
<testsuites name="feishu-calendar-suite" tests="3" failures="1" time="15.6">
  <testsuite name="feishu-calendar-suite" tests="3" failures="1">
    <testcase name="create-single-event" time="2.3" />
    <testcase name="batch-reschedule" time="8.1" />
    <testcase name="timezone-handling" time="5.2">
      <failure message="check-timezone.py 验证失败">
        期望 UTC 时间 01:00，实际写入 09:00（未做时区转换）
      </failure>
    </testcase>
  </testsuite>
</testsuites>
```

---

## 六、与 Sage 已有系统的集成

### 6.1 Harness + TaskRecord

`sage test` 产生的每个 case run 也写 TaskRecord（`session_type: HarnessRun`），便于追踪回归趋势：

```json
// workspace/metrics/<task_id>.json（HarnessRun）
{
  "task_id": "01HZ...",
  "session_type": "HarnessRun",
  "suite": "feishu-calendar-suite",
  "case": "create-single-event",
  "success": true,
  "harness_result": {
    "passed": true,
    "score": 1.0,
    "criteria": [...]
  },
  ...TaskRecord 标准字段...
}
```

累积的 HarnessRun 记录可以：
- 检测 token 消耗是否在增加（知识积累应降低消耗）
- 追踪成功率趋势
- 检测模型更换后的能力回归

### 6.2 Harness + Craft

Agent 通过 Craft 积累的 SOP 会改变其行为——Harness 是验证 Craft 是否真的提高了效率的闭环工具：

```
task_1（无 Craft）→ Harness: pass, tokens=3200
  ↓
Craft 创建（batch-calendar-update v1）
  ↓
task_2（有 Craft）→ Harness: pass, tokens=1800  ✓ 效率提升 44%
```

### 6.3 Harness + DaemonLoop

交互会话中，Stop hook（运维 Harness）和测试 Harness 共用同一套机制，但有不同的 `session_type`：

| SessionType | Stop hook 目的 | 触发方式 |
|---|---|---|
| `UserDriven` | 可选质量门控（prod Harness） | 用户正常使用 |
| `HarnessRun` | 功能验证（test Harness） | `sage test` 命令 |
| `WikiMaintenance` | 无质量门控 | Daemon IDLE |

---

## 七、Runeforge Harness Pattern

Runeforge 是 Sage 最典型的上层使用方。它的 Harness 模式：

```
Runeforge Task 提交
  │
  ▼
DaemonLoop PROCESSING
  │ (agent 执行)
  ▼
Stop hook（Runeforge 注入的 eval.sh）
  ├── 检查 workspace artifacts（代码/文档/数据）
  ├── 运行单元测试 / lint
  ├── 验证 API 合约（JSON Schema）
  │
  ├── exit 0 → 任务通过，返回 Runeforge
  └── exit 2 → stderr 注入 Agent，Agent 自我纠正
              （最多 max_turns 轮，超限后 exit 1 标记 timeout）
```

这使 Runeforge 能定义**领域特定的完成标准**，而不需要修改 Sage 内核——一行 `hooks.Stop` 配置，Sage 就变成了一个有质量门控的 Agent。

---

## 八、实现任务

### P0（随 v0.9.0 Hook 系统一起交付）

- [ ] `Stop` hook 正确传递 `turn_count`、`session_id`、`task_id` 到 stdin JSON
- [ ] exit 2 协议：Stop hook 失败时，stderr 作为 steering message 注入 agent loop，让 Agent 再次尝试
- [ ] `max_turns` 超限时，Stop hook 失败不再注入，改为 `RunError`（避免无限循环）
- [ ] command hook timeout：超时的 eval 脚本强制 kill，记录为 Harness 系统错误

### P1（v0.8.0 sage test 命令）

- [ ] `sage test --suite <path>` 命令解析（HarnessConfig YAML）
- [ ] Suite runner：顺序/并发执行 cases，每个 case = 独立 DaemonLoop 实例（workspace 隔离）
- [ ] Workspace setup：每个 case 启动前可 copy/override workspace 文件（`setup` 字段）
- [ ] 三种 reporter：terminal（彩色）/ json / junit
- [ ] HarnessRun TaskRecord 写入（session_type 字段）
- [ ] `--max-turns` flag 覆盖 suite 配置

### P2（v0.9.x 扩展）

- [ ] `prompt` criteria 类型：调用小模型做语义评分（返回 0.0-1.0 分数）
- [ ] `agent` criteria 类型：启动子 Sage 实例执行验证脚本
- [ ] Criteria 权重：加权平均 score，而非全部通过才算 pass
- [ ] 回归对比：`sage test --compare <baseline.json>` 对比两次运行的 token 消耗和成功率差异
- [ ] Suite 组合：`include` 字段引入其他 Suite（共享 criteria 库）

---

## 九、设计边界（不做的事）

| 不做 | 理由 |
|------|------|
| Harness 管理 Agent 内部状态 | eval 脚本只访问 workspace 文件，不读取 Agent 内存 |
| Harness 替代 TaskRecord | TaskRecord 是运营指标（无 ground truth），Harness 是质量门控（有 ground truth）—— 互补，不替代 |
| Harness 内置语义评估模型 | `prompt` criteria 由上层注入模型，Sage 不内置 judge 模型 |
| Harness UI | `sage test` 输出到 stdout，UI 由 Runeforge / CI 系统消费 |
