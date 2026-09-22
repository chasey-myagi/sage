# @sage/plugins

插件提交申领单，编译器在总预算下分配档位，再组装成这一轮的视图。

## Model Experience

### What the model sees

模型看到的是一条阶段头 user 消息，加上追加在最后一条 user 消息末尾的 text block。阶段头里每张 epoch-head 申领单是一段，段与段之间空一行。尾部 `tail` 与 `tail-if-changed` 用同一格式，各占一个 text block，不另开消息：

```text
<claim id="…" tier="…">
正文
</claim>
```

`none` 档的申领单不进视图。被淘汰的工具块仍留下 tool_use，配对的 tool_result 正文换成一行桩，isError 不变：

```text
[#<chunkId> · <toolName> · <摘要> · <n>k tokens · omitted · expand("<chunkId>") to retrieve]
```

摘要是这条 tool_use 输入的紧凑单行，最长 80 个字符，更长就截断并加上 `…`。`<n>k` 是该块原文 token 数除以 1000，保留一位小数。模型要当时的原文时调用 `expand`，参数 `id` 就是桩里的 chunkId。这个工具按块 id 从 Log 取回当时的 tool_result 或文本消息，不重新执行原来的工具。

expand 率 = 清单 `budget.expands` / `budget.omitted`。`expands` 是截至本轮、路径上名字为 `expand` 的 tool_use 次数；`omitted` 是本轮档位为 none 的块数。

### Token effect

进视图的申领单按所选档位计 token，总和受预算约束。清单里每张申领单都有 tier、reason、tokens；`budget` 记下 total、used、overrun。

历史按块估算。token 影响等于淘汰之后仍保留的块的 token 合计；降到 none 的工具块不再按原文占用剩余预算。清单里该块的 `tokens` 改记桩文本本身，`originalTokens` 保留原文估算。`budget` 除了 total、used、overrun，还有 omitted 和 expands。

### KV cache effect

阶段内，前缀逐字节冻结，只在尾部追加。缓存断点由视图给出，适配器只翻译：

```text
["head", {afterMessage: 冻结前缀最后一条}, {afterMessage: 最后一条}]
```

`head` 盖住 system 和工具 schema。第二个断点落在本阶段第一份清单覆盖到的历史末尾（epoch 头加上当时的 seq 上限）。第三个断点落在当前最后一条消息。新块从那个 seq 之后才进入淘汰，近期窗口也只在这些新块里数。

开不开新阶段由这道不等式批准：

```text
(旧视图 − 新视图) × 读价 × 预计剩余轮数  ＞  新视图 × 写价 − 旧视图 × 读价
```

口径：

- 每轮节省 = (旧 − 新) × 读价
- 重建额外成本 = 新 × 写价 − 旧 × 读价。减掉的那一项是因为重建的那一轮本来也要付旧视图的读价，不能重复计
- 回本轮数 = 额外成本 ÷ 每轮节省，向上取整
- 额外成本不大于 0 就是立即回本

读价、写价来自目录里该模型的 `cacheRead / input` 与 `cacheWrite / input`。目录没有定价时退回 0.1 与 1.25。旧视图 token 用上一轮 usage 实测锚定，新视图先估算再乘以 measured/estimated。省下的比例小于 `minSavingsRatio`（默认 0.2）就不开新阶段。视图估算超过上下文窗口的 0.9 时强制重开一次；这是安全阀，不是策略。重开后仍然超限就记在 `budget.overrun`，不再为同一道安全阀重复重开。

## 路由

`contextCompiler({ route })` 只在本轮决定重开阶段时调用。策略没装，或者 `rebuild` 为 false，一次都不会进策略。

传给策略的候选默认是当前 provider 的 `listModels`。要跨 provider，调用方在策略里换成已装配 provider 的目录并集。`costTierPolicy` 只看 ladder 上的型号，目录里定不了价的跳过。

`sensitivePolicy` 看的是这次新视图的块，不是整段历史。tier 为 `none` 的块不触发规则。命中的 access key 写成 `sensitive:<key>`，对应 provider 从候选里拿掉。`withSensitiveFilter` 先裁候选，再跑里面的策略。`costTierPolicy({ pick: "cheapest-allowed" })` 在剩下的 ladder 里选 input 价最低的。

选中的型号写在 `epochStart.model`。策略 reason 追加到清单的 `epochDecision.reason`，形如 `;route:<reason>`。适配器切换和 `model_change` 由内核落盘。

## 第一方申领单

- 系统提醒：模型在最后一条 user 消息末尾看到一行提醒；落点 `tail`；只有 line 档。
- AGENTS.md：模型在阶段头看到工作区 `AGENTS.md` 的全文，预算不够时降到前 40 行加 `…`；落点 `epoch-head`；档位 full / summary。工具 path 碰到的子目录 `AGENTS.md` 挂在尾部，正文没变就不重写全文；落点 `tail-if-changed`。
- 工具目录：模型在阶段头看到每个延迟工具一行「名字 — 说明」，full 档在这组行之后附上各自的 inputSchema；落点 `epoch-head`；档位 line / full。目录为空则不出现，也不改工具头。
- 进度账本：模型在尾部看到 Goal / Plan / Decisions，钉住不降；落点 `tail-if-changed`；只有 full 档。没有 `update_ledger` 调用则不出现。
