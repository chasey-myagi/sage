# 打分器调用计数只数真正发出的请求

## Problem

[plugins 字符串协议整理](./2026-09-24-plugins-string-protocols.md) 留下两处口径不一致：

- `jevScorer` 缺 API key 时直接返回 `no-api-key`，没有发请求，但没有设置 `requested: false`，重建打分的 `manifest.scorer.calls` 记 1。
- 入场打分遇到任何 fallback 都记 1 次调用，包括模型打分器的 `no-adapter`；重建打分同一情况已经按 `requested: false` 记 0。

同一个 `calls` 字段，同一个「没发请求」的事实，在两条路径、两个打分器上记出不同的数。`runHealth` 的 `scorerCalls` 与 `entryScored.calls` 把它们加总，健康度里的调用数因此混进了从未发生的请求。

## Decision

`calls` 只数真正发出的打分请求，判断读结构化字段 `ScoreResult.requested`，不读 fallback 文字。

- `jevScorer` 缺 key 时返回 `{ decisions: [], fallback: "no-api-key", requested: false }`。
- `score-common.ts` 新增 `requestCount(result)`：`requested === false` 为 0，否则为 1。重建打分的 `reportOf` 与入场打分的 fallback 分支都用它，两条路径的口径写在一处。
- 入场打分在打分器抛错时仍记 1：抛错发生在哪一步不可知，与重建打分把抛错折成 `fallback: "error"`、不带 `requested` 的处理一致。
- 入场打分遇到 `requested: false` 仍写 `entryScored`（`calls: 0`，各档为 0），与重建打分在同样情况下仍写 `scorer`（`calls: 0`、`fallback: true`）对称。

文档同步：`ScoreResult.requested`、`ScorerReport.calls`、`EntryScoredReport` 与 `RunHealth.scorerCalls` 的 JSDoc，`score-entry.ts` 文件头，`packages/plugins/README.md` 的重建打分、jev 与入场打分三段，`packages/core/README.md` 的健康度口径。

## Alternatives considered

入场打分遇到 `requested: false` 时干脆不写 `entryScored`：清单上就看不出这一轮试过打分、退回了，而重建打分在同样情况下是写报告的，两边又不对称。

按 fallback 字符串（`no-api-key`、`no-adapter`、`no-previous-request`）判断：正是上一轮刚去掉的字符串协议。

把 `calls` 改成数 HTTP 次数（jev 分批、429 重试）：`calls` 一直是「一次 `score()` 调用」的粒度，[jev 打分器](./2026-09-23-jev-scorer.md) 明确批内 HTTP 次数不另计。改粒度是另一件事。

## Consequences

这是有意的行为修改。`packages/plugins/test/scorer/jev.test.ts` 的「falls back when the api key is missing and does not fetch」用整对象 `toEqual` 钉住了缺 key 的返回值；它钉的是被淘汰的口径（没有 `requested` 字段，于是被记成一次调用），这次只在期望对象里加上 `requested: false`，其余字段与 `attempts === 0` 的断言不变。

已有 Log 里，缺 key 的 jev 重建打分与无 adapter 的入场打分记的是 `calls: 1`，重放旧 Log 算出的 `scorerCalls`、`entryScored.calls` 会比新口径多。首个 tag 之前不做迁移。

`shadow.calls` 的口径没有动：它数的是带 `shadow` 对象的清单条数，不是请求数，影子打分缺 key 时照样计 1。要不要对齐另议。

## Testing

红灯先行，新增两条用例，改代码前都红（`calls` 期望 0，实际 1）：

- `packages/plugins/test/scorer/jev.test.ts`：「records zero rebuild scorer calls when the api key is missing」，断言 `manifest.scorer` 为 `{ calls: 0, usage: 全零, fallback: true }` 且没有 fetch。
- `packages/plugins/test/regressions/issue-73.test.ts`：「records zero entry calls when the model scorer has no adapter, as rebuild does」，用真实 `modelScorer({ adapters: {} })`，断言块保持 full、`manifest.entryScored` 的 `calls` 为 0、`runHealth` 的 `entryScored.calls` 为 0。

改完后 `pnpm check` 退出码 0。除上面那一处期望值，没有改其他断言，没有删用例。
