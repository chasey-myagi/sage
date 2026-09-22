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

`none` 档不进视图。

### Token effect

进视图的申领单按所选档位计 token，总和受预算约束。清单里每张申领单都有 tier、reason、tokens；`budget` 记下 total、used、overrun。

### KV cache effect

epoch-head 在同一阶段内逐字节冻结，阶段不重开，前缀不变。`tail` 每轮重算，`tail-if-changed` 只在正文变化时换新文本，两者都只影响消息尾部。

## 第一方申领单

- 系统提醒：模型在最后一条 user 消息末尾看到一行提醒；落点 `tail`；只有 line 档。
- AGENTS.md：模型在阶段头看到工作区 `AGENTS.md` 的全文，预算不够时降到前 40 行加 `…`；落点 `epoch-head`；档位 full / summary。工具 path 碰到的子目录 `AGENTS.md` 挂在尾部，正文没变就不重写全文；落点 `tail-if-changed`。
- 工具目录：模型在阶段头看到每个延迟工具一行「名字 — 说明」，full 档在这组行之后附上各自的 inputSchema；落点 `epoch-head`；档位 line / full。目录为空则不出现，也不改工具头。
- 进度账本：模型在尾部看到 Goal / Plan / Decisions，钉住不降；落点 `tail-if-changed`；只有 full 档。没有 `update_ledger` 调用则不出现。
