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
