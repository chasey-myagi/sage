---
name: sage-prose-standard
description: 在 sage 仓库写、审、删减注释与文档时使用——文件头契约、导出 JSDoc、函数内注释、包 README、Agent Note、CLI 与模型可见文本各该写什么、不该写什么。Use when writing or reviewing comments, JSDoc, READMEs, Agent Notes, or model-visible strings in sage.
---

# sage 的文字规范

写到足以保住契约为止，然后删掉推理过程、重复和修饰。契约指调用方、实现方、生产方或消费方依赖的义务：前置条件、后置条件、不变量、失败方式、兼容承诺。这是指引，不是脚本。

语言：代码注释、JSDoc、日志与错误信息用英文；README、设计文档、Agent Note、commit 与 PR 用中文。

## 各处该写什么

- **文件头（G7）。**模块职责；它对调用方的保证；依赖了谁、刻意不依赖谁；`Intentionally not in the Log:` 列出故意不进 Log 的东西。不超过 25 行。不写逐个 PR 追加的行为清单，那些写到实现它的函数上。
- **导出 JSDoc。**每个导出的函数、类、类型、常量都有。首句说它是什么或做什么。然后只写签名表达不了的：返回值之间的区别、会抛什么、副作用、谁拥有返回的对象、调用时机、取消行为、默认值。参数名和类型已经说清楚的不写 `@param`。
- **类型字段。**单位（token、字节、毫秒）、取值范围、缺省时的含义、谁写谁读。
- **领域术语。**chunk、tier、claim、epoch、stub、manifest 在定义它的类型上用一两句解释，别处直接用。
- **函数内注释。**只写代码表达不了的：不变量、竞态与顺序、安全边界、出人意料的失败行为、「为什么不用更显然的写法」。删掉复述代码和叙述控制流的注释。
- **测试注释。**只解释不显然的测试设计：为什么需要这个 fixture、为什么走真实入口、为什么这样间接观察。用例名本身描述行为。
- **包 README。**消费者要知道的：配置、语义、失败、限制、扩展点；`## Model Experience` 三小节写清模型看到什么、token 影响、KV 缓存影响。
- **Agent Note。**Problem / Decision / Alternatives considered / Consequences / Testing。保留独有的理由、否掉的方案、后果、验证证据。已实现的写成现在时的事实，不留计划清单。归档后冻结。
- **模型可见文本与 CLI 输出。**措辞就是行为：改 system prompt、工具描述、claim 文本、usage 文本，要有对应测试或快照覆盖，并在 PR 里说明。
- **错误信息。**说出失败的对象或路径、违反的规则，以及不显然时怎么改。

## 删什么

- 复述代码的注释：`// increment counter`。
- 评审历史与过程：「review 后改成…」「之前是…现在是…」。
- 裸 issue 编号 `(#12)`：换成一句结论，需要出处就链到 `docs/notes/` 的文件。
- 同一解释的第二份：一个事实一个归属地，其他地方链接过去。关键契约可以在使用点重复。
- 隐喻和修饰性形容词。

## 用词

`contract`、`boundary`、`shape`、`surface`、`seam` 用之前先问：有没有更准确的词直接说出对象？写 `response fields`、`JSON validation`、`ESM exports`，不写 `response shape`、`validation boundary`、`module shape`。`seam` 在本仓有专指（View seam、测试缝），只用在这两处。

## 删减时保住完整命题

改写一段话之前，先列出它的每个命题：主体与动作、条件与时机、情态（must / may / never）、否定保证与例外、所有权、副作用、失败方式。所有事实都还在、并且更清楚，才算改好。字数少了本身不是改进。

## 流程

1. 确认范围。只在请求的文件或 diff 里工作。
2. 读拥有这段行为的代码，再判断文字是否准确。
3. 每一处归类：保留、补写、删减、重写、暂缓。
4. 先改归属地，再改引用它的地方；学到一条新规则，回头检查同类段落。
5. 跑 `pnpm check`；模型可见文本改动跑对应测试。报告改了哪些、刻意保留了哪些、暂缓了哪些。
