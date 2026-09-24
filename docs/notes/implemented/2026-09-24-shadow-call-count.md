# 影子打分的 calls 只数真正发出的请求

## Problem

[打分器调用计数](./2026-09-24-scorer-call-count.md) 把重建打分与入场打分的 `calls` 统一为「只数真正发出的请求」，但留下了 `runHealth` 的 `shadow.calls`：它数的是带 `shadow` 对象的清单条数。`--shadow-scorer jev` 缺 key 时 jev 直接返回 `no-api-key`、没发请求，清单上照样写 `shadow`，健康度记 1 次调用。

清单的 `shadow` 记录里也没有结构化的「发没发请求」信息，core 只能按条数计。

## Decision

`manifest.shadow` 新增 `calls`，与 `scorer.calls`、`entryScored.calls` 同一口径，`runHealth` 的 `shadow.calls` 改为加总它。

- plugins `score-shadow.ts`：正常返回时 `calls = requestCount(result)`，`requested: false` 为 0，其余为 1。打分器抛错记 1，与入场打分一致（抛在哪一步不可知）。读 Log 或构造请求失败时记 0：此时还没调用打分器。构造请求从内层 `try` 挪到外层，fallback 文字不变。
- core `ShadowReport` 加必填 `calls: number` 并写 JSDoc。`log/kinds.ts` 校验 `calls` 为有限数或缺省。`health.ts` 的 `shadowHealth` 读 `shadow.calls`，缺失或非有限数加 0。
- 文档：`packages/core/README.md` 健康度口径、`packages/plugins/README.md` 影子打分段、`apps/sage/README.md` 的 `--shadow-scorer` 与 `sage stats` 两段、`score-common.ts` 的 `requestCount`、`ScoreResult.requested`、`RunHealth.shadow` 的 JSDoc。

## Alternatives considered

在 `shadow` 上记 `requested?: false`，与 `ScoreResult` 同形：core 计数时还得把「缺省」翻译成 1，和另外两份报告写 `calls` 的做法不对称。

`kinds.ts` 把 `calls` 设为必填：已有的 bench 会话 jsonl 重开时会整份报 `LogCorruptedError`，`sage stats` 读不了旧文件。可选校验只放宽了一个新字段，`calibration.head` 已有同样的先例。后来改为必填，见 Consequences。

core 按 `shadow.fallback` 文字判断：字符串协议，上一轮刚去掉。

## Consequences

这是有意的行为修改：`shadow.calls` 从「清单条数」变为「请求数」。

旧 Log 的 `shadow` 没有 `calls`。最初按可选字段校验、健康度计 0，让旧会话还能重开；复审指出这是兼容垫片，违反 G4，也与字段改名不迁移的口径不一致，已改为必填：`log/kinds.ts` 要求 `calls` 为有限数，旧会话重开时报 `LogCorruptedError`，`sage stats` 读不了它们。首个 tag 之前不迁移。

`packages/core/test/health.test.ts` 的「counts each shadow manifest once and sums its usage quadruple」输入的两条 `shadow` 是旧形状（没有 `calls`），新口径下会算出 0。这次只在两条输入上补 `calls: 1`（一条有 usage、一条是抛错的 `boom`，按新口径都发过请求），期望值 `calls: 2` 与其余断言不变，用例标题不变。

## Testing

红灯先行：`packages/plugins/test/scorer/jev.test.ts` 新增「records zero shadow calls when the api key is missing and one when it is sent」，缺 key 的影子 jev 断言 `manifest.shadow.calls` 为 0、`runHealth(...).shadow.calls` 为 0、没有 fetch；带 key 时均为 1。改代码前实际为 `manifestCalls: undefined`、`healthCalls: 1`。另在 `packages/core/test/health.test.ts` 新增「adds nothing for a shadow manifest that sent no request or lacks calls」。

`pnpm check` 退出码 0（88 个测试文件、436 个测试）。
