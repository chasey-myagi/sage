---
name: sage-find-simplifications
description: 在 sage 仓库寻找可以删除、合并、下沉的代码与文档时使用——没有生产调用方的导出、复制出来的工具函数、只有测试在用的 API、手写的基础设施、堆积的注释；产出有证据的 proposed Agent Note 或直接的清理 PR。Use when surveying sage for dead, duplicated, speculative, or hand-rolled code.
---

# 找简化点

目标是删掉或合并真实存在的东西，并拿出证据证明它现在的代价大于收益。几条证据扎实的候选胜过一堆猜测。这是指引，不是清单。

## 先读

- [`AGENTS.md`](../../../AGENTS.md)：尤其是分层不变量、契约红线和「预发布不留兼容层」。
- [`docs/design/context-compiler.md`](../../../docs/design/context-compiler.md)：判断 `packages/` 里的东西前先知道它为什么存在。
- `docs/notes/implemented/`：有意为之的设计。两个 adapter、两种打分应用方式、影子打分都是有记录的决定，要推翻需要新证据。

## 强候选

- 导出符号、选项、Log 字段、CLI 旗标没有生产调用方，只有测试或文档在用。
- 同一个工具函数、常量、类型在多个文件里各定义一份。
- 同一个事实有两个来源（例如内存状态与 Log 各记一份）。
- 手写的基础设施，Node 内置或成熟依赖已经提供（参数解析、重试退避、glob、diff），换掉能删掉实现和它专属的测试。
- 超长函数里可以拆出、且拆出后有名字能说清楚的一段。
- 按时间堆积的文件头、复述代码的注释、裸 issue 编号。

不够格的：只是「看起来复杂」而没有调用点证据；删一个拼写；推翻有 Agent Note 支撑、而新证据并不更强的设计。

## 证明或否决

每个候选先把使用者分类：

- 生产：`packages/*/src`、`apps/*/src`、`apps/sage/bin`、`scripts/` 里被闸门调用的脚本。
- 非生产：`test/`、README、`docs/`、Agent Note。

用 `rg` 搜精确符号名、`.name(` 与 `name(`、字符串字面量、CLI 旗标名，然后读调用点。有生产调用方，删它就是功能决定而不是清理，降级为建议。

## 产出

- 小而确定的清理：直接开一个 `refactor/` PR，测试断言不动。
- 需要取舍的：`docs/notes/proposed/<日期>-<主题>.md`，写 Problem（现状与使用者证据，生产与测试分开）、Proposal（具体删什么、合并什么，连同测试、文档、README）、What we give up（最强的反对理由）、Acceptance criteria、Risks。
- 局部的小问题不值一份 Note 时，写 `TODO(<标签>): <为什么可以改、怎么改>`。

## 报告

列出调查过的范围、每个候选的证据与结论（采纳 / 提议 / 否决）、刻意排除的部分、跑过的检查。
