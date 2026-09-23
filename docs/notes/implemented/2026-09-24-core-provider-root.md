# 分层自检禁止 core 源码 import provider 主出口

## Problem

`AGENTS.md` 与 `docs/development.md` 都写着 core 只经 `@sage/provider/contract` 看 provider，碰不到任何 HTTP 代码，并说由 `scripts/check-layers.mjs` 执行。实际规则 3（`packages/core/**`）的 deny 表只有 `@sage/plugins`、`@sage/tools` 和 `apps/**`。`packages/core/src` 下的文件 import `@sage/provider`（主出口，带两个 adapter 与 HTTP 实现）照样通过自检。文档声称的不变量没有机器守着。

## Decision

规则 3 的 deny 表加一个 token `core-src-provider-root`。`matchesDeny` 对它的判断是：import 的说明符恰好是 `@sage/provider`，并且文件在 `packages/core/src/` 下。

- 只作用于 `packages/core/src`。core 的测试（`packages/core/test`）仍可经主出口 import 真实 adapter，注入 fetch 驱动整条路径，例如 `test/session/incomplete-stream.test.ts`。
- 只拦主出口本身。`@sage/provider/contract` 不受影响。
- 自检测试的「合法夹具」原先让 `packages/core/src/index.ts` import `@sage/provider`。它钉住的正是过宽的旧规则，在新规则下本来就该判违规，所以改为 import `@sage/provider/contract`。这是有意改变被测行为，不是放宽断言。

## Alternatives considered

新增一条独立规则编号，专管 core 源码对 provider 主出口的 import。不选。`ruleFor` 对每个文件只取第一条匹配的规则：新规则若排在规则 3 之前，`packages/core/src` 下既有的违规（例如 import `@sage/plugins`）会改报新编号，已有测试与报告里的「违反规则 3」随之变化；排在之后则永远轮不到它。要么改 `ruleFor` 为多规则合并，要么接受编号漂移，两者都比在规则 3 里加一个 token 动得多。

连 core 的测试一起禁。不选。core 的会话级测试需要经真实 adapter 验证 provider 与 core 的衔接（传输标记、未完成的流、流内 error 事件），这些只能从主出口拿到 adapter。测试不进产物，不破坏「core 运行时碰不到 HTTP 代码」这一条。

## Consequences

`packages/core/src` 下再出现 `import … from "@sage/provider"` 时，`pnpm check:layers` 报「违反规则 3」。core 需要 provider 的新东西时，先把它放进 `@sage/provider/contract`（类型与纯函数），再从那里 import。

规则表里多了一个带路径条件的 token，读规则 3 时要同时看 `matchesDeny` 里那两行判断。

## Testing

`scripts/__tests__/check-layers.test.ts` 新增用例「rejects the provider root export from core sources but not from core tests (rule 3)」：`packages/core/src/adapter.ts` import 主出口报一条规则 3 违规，`packages/core/src/contract.ts` 经 contract 与 `packages/core/test/adapter.test.ts` 经主出口都不报。修改规则前该用例红（`violations` 为空），修改后绿。`pnpm check` 全绿。
