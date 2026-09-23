---
name: sage-code-review
description: 在 sage 仓库评审 PR 或一组本地提交时使用——把评审者带到本仓的标准（AGENTS.md 规范、契约红线、防御模式、Agent Note、闸门），列出代码本身看不出来、必须人工核对的项目。Use when reviewing a sage pull request or branch.
---

# 评审 sage 的改动

这是指引，不是完整清单。先确认 PR 的真实 base 与 head，`git diff <base>...<head> --stat` 看范围，再读 diff 和足够多的周边代码。优先级：正确性、Log 可重放、会话生命周期、安全，然后才是风格。一条有证据的阻塞意见胜过一串小毛病。

## 标准出处

- [`AGENTS.md`](../../../AGENTS.md)：分层不变量、测试规则、契约红线、代码规范、注释与文档。
- [`docs/defensive-patterns.md`](../../../docs/defensive-patterns.md)：失败归一化、重试分层、append 串行、隔离失败即退出、前缀字节稳定。
- [`docs/development.md`](../../../docs/development.md)：常见改动的落点与规范条款的理由。
- [`sage-prose-standard`](../sage-prose-standard/SKILL.md)：注释、JSDoc、README、Agent Note 的写法。
- `docs/notes/implemented/`：既有设计决定。和某份 Agent Note 冲突是设计讨论，不是自动否决。

## 阻塞项

1. **闸门绿。**`pnpm check` 全过；分层自检的规则表只能随 Agent Note 一起改。
2. **契约红线。**新的模型可见输入有对应的 Log 条目（L4）；同一事实只有一个来源（L5）；新 entry kind 已注册且 `docs/log-manifest.md` 已重新生成。
3. **测试是证据。**行为修改有一个在修复前会红的测试；断言只打在 view / manifest / log 上；没有放宽已有断言、删用例、加 skip。纯重构 PR 不改任何测试断言。
4. **失败处理。**工具与 provider 的失败以值返回；归一化没有丢掉分类字段；安全开关做不到时退出而不是降级。
5. **文档同步。**改到的文件头契约、导出 JSDoc、包 README（含 Model Experience 三小节）、CLI usage 与本次行为一致。非平凡改动带 Agent Note。
6. **规范硬项。**无新增 `any`；新增 `as` 有理由；无新增魔数与复制出来的工具函数；逻辑不读 `reason` 字符串。

## 人工核对

- **契约两侧。**改了接口就追到每个调用方和实现方：返回值区别、抛错、取消、所有权是否都对得上。
- **模型视角。**读模型实际收到的 system prompt、工具 schema、工具结果、claim 文本。多了模型用不上的概念就指出来；稳定文本要有快照或端到端覆盖。
- **缓存与前缀。**改编译器或视图时，确认已进入视图的字节不会在阶段内被改写；`cacheBreakpoints` 的位置仍然合理。
- **重放。**只拿 Log 能否还原每一轮请求；新增的默认值、hash、配置是否落盘。
- **上限覆盖最终产物。**截断、预算、字节上限要作用在最终发出去的内容上（含包装与元数据）；试一试刚好等于上限、单块超限、多字节字符。
- **真实入口。**CLI 行为用 `runCli` 或 bin 路径测，不只测内部函数。
- **范围。**一个 PR 一件事；顺手重构与行为修改混在一起时要求拆开。
- **必要性。**每个新选项、新导出、新抽象都要有当前的生产调用方；只有测试在用的公开 API 是扩张。

## 写评审意见

每条写清：缺陷、位置（`file:line`）、影响、证据。阻塞与建议分开。已经被绿闸门保证的事项不重复提。收到评审时逐条核实，按技术理由修改或反驳，不做表态式认同。
