# 开发规范、防御模式与项目 skill

## Problem

仓库有分层不变量、测试规则和契约红线，但没有写到代码层面的规范。2026-09-24 的代码体检发现：源码约 90 个文件都有 G7 文件头，函数级注释几乎为零；`compiler.ts` 的文件头 113 行，是按 PR 追加的行为清单；`isRecord` 复制在二十多个文件里，sleep 与退避公式、用量四元组类型各有多份；`40000`、`80`、`0.9` 等数字散在多处；有逻辑靠 `reason.includes(...)` 判断；CLI 旗标清单在 `main.ts` 里写四遍、`bench.ts` 另一套。另有两处与代码不符的文字：AGENTS.md 把依赖方向写成 `provider → core`（实际 core 依赖 provider 的 contract），README 说 Log 有持久化水位（HWM），而设计 L1 已经去掉了它。

新贡献者和 agent 没有一处能查到「在这里代码该怎么写」，评审也没有可引用的条文。

## Decision

规范分三层放：

- `AGENTS.md` 新增「代码规范」「注释与文档」「交付」三节，每条一句、可在评审里直接引用；改正依赖方向。
- `docs/development.md` 讲工具链、源码不编译直接跑带来的约束、包的布局、常见改动的落点、不显然条款的理由、闸门与提交格式。
- `docs/defensive-patterns.md` 收九条本仓真踩过的坑，每条链到当时的 Agent Note。

项目 skill 放在 `.agents/skills/`，`.claude/skills` 是软链：`sage-code-review`、`sage-prose-standard`、`sage-pre-push-checks`、`sage-find-simplifications`。结构参照 deepseek-harness 的同名 skill，内容按 sage 的规模和约定重写。

README 的 Log 一行改成与 L1 一致。

## Alternatives considered

直接照搬 deepseek-harness 的规范。它为 30 万行、两百多个包、双语文档和 Cordis 插件体系设计：导出 JSDoc 生成器、双语配对、快照归属、标签分类、栈式 PR 流程，sage 都用不上，照搬只会留下没人执行的条文。取了它的原则（边界验货、失败归一化保留分类、注释只写契约、导出必有 JSDoc、依赖优于手写、空 catch 要说明），丢掉它的机制。

把规范写成独立的长文档、AGENTS.md 只放链接。agent 每次都会读 AGENTS.md，不一定会跟链接；可引用的条文必须在 AGENTS.md 里，理由和例子才放到长文档。

先加 lint 规则或自检脚本再写规范。导出 JSDoc 的自检要等现有代码补齐之后才能打开，否则第一天就红；这份 PR 只定条文，自检在代码清理完成后单独加。

## Consequences

条文比现有代码严格：现有代码有大量不合规处（超长函数、复制的工具函数、缺 JSDoc、魔数）。后续按包逐个 PR 清理，每个 PR 都以这份规范为验收标准；清理完成后加导出 JSDoc 自检，让「每个导出符号有 JSDoc」由闸门执行。

契约红线 L4 原写「system prompt 只在偏离默认时存全文」，而 `session.ts` 的实现是「首见或与上一条 `request_header` 的 hash 不同就存」。代码里没有「默认 prompt」这个概念，实现本身就是正确的内容寻址，所以改文字：AGENTS.md 与 `docs/design/references.md` 的 L4 都改为「system prompt 与 tool schema 都按 hash 存，首见或变化时才存全文」。

## Testing

纯文档改动。`pnpm check` 通过（README 自检与其余闸门不受影响）。
