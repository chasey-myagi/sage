# 重建成本计入重建后代价，阶段活过承诺的回本才按回本重开

## Problem

真实运行里一个任务会重开好几个阶段，每次重开都按全价重编前缀，总成本可能高过不压缩。成本不等式有三处问题：

- 视界远大于阶段寿命。`remainingTurnsFor` 给的是整个任务的剩余轮数，回本要 15、20 轮的重开也会被批准；几轮后同一道不等式又批准下一次重开，上一次的写还没赚回来就被作废。这是重开过多的根因。
- 没算重建后代价。压缩之后模型用 `expand` 把原文拿回来，这部分没进账。
- 费率来源看不出来。清单只有一个 `;pricing:fallback` 后缀；目录里自动前缀缓存的型号把写价记成 0，靠 `ratesFor` 的 `max(cacheWrite, input)` 才变成 1.0，目录和价格页的口径不一致。

## Decision

不等式：

```text
(旧 − 新) × 读价 × 剩余轮数  ＞  新 × 写价 − 旧 × 读价 + 预计取回 × 写价
```

- **阶段承诺的回本。**按回本批准的重开，承诺新阶段至少活过它自己的 `paybackTurns`。承诺值就是阶段第一份清单（批准重开的那一份）的 `epochDecision.paybackTurns`。`freeze.ts` 的 `epochCommitment` 从那份清单读出 `turnsSinceEpochOpen` 与 `committedPaybackTurns`。已过轮数小于承诺值时，按回本批准的重开改成不重开，原因 `payback-pending:<已过><<承诺>`，trigger 是 `no-rebuild`。阶段 0、立即回本开的阶段（回本 0）、安全阀开的阶段（`chooseEpoch` 按 `view_epoch` 原因置 0）没有承诺。立即回本与上下文窗口安全阀不受限。判据全部来自 Log，重放结果相同。没有新的魔数。
- **重建后代价。**`postRebuildCost = 预计取回 × 写价`，计入 `extraCost`，预计取回不超过 旧 − 新。`log-read.ts` 的 `expandResultTokens` 从 path 读出每次成功 `expand` 的结果 token。有观测时 `{source: "observed", expandsPerEpoch, tokensPerExpand}`，预计取回 = 每阶段次数 × 平均 token；这是上界，阶段 0 因预算省略触发的 `expand` 也被算进去，而且全按写价计。没有观测时用先验 `{source: "prior", share: PRIOR_EXPAND_SHARE}`，预计取回 = 0.1 × (旧 − 新)，让回本轮数多出 `0.1 × 写价 ÷ 读价` 轮。
- **费率回到目录真值。**价格页写明「未命中按输入价、无创建费」的内置条目（OpenAI 三款、DeepSeek 两款、Kimi k2.6 与 k2.7-code、百炼四款）把 `cacheWritePerMTok` 改成等于 `inputPerMTok`。`ratesFor` 只在定价缺失（或输入价为 0）时用 `DEFAULT_READ_RATE` 0.1 与 `DEFAULT_WRITE_RATE` 1.25；`max(cacheWrite, input)` 保留，兜住用户在 providers.json 里写的 0。`Rates.source` 与清单 `epochDecision.rateSource` 取 `pricing` 或 `default`。
- 清单 `epochDecision` 新增 `rateSource`、`postRebuildCost`、`expandEstimate`、`turnsSinceEpochOpen`、`committedPaybackTurns`，core 的 `EpochDecisionReport` 同步。`kinds.ts` 的校验与 entry kind 文档不变，`docs/log-manifest.md` 不需要重生成。
- `replayOffline` 的 policy 新增 `claims`，回放可以带上录制时的申领单，从而逐字段复现录制的清单数字。

## Alternatives considered

为什么用「承诺的回本」当间隔，而不是把视界改成 min(剩余轮数, 预计阶段寿命)。阶段寿命要靠历史阶段长度来估，第一次重开时没有任何历史，只能再引入一个先验常数；而且寿命本身又取决于这条规则，估计会自我循环。承诺的回本是 Log 里已有的事实，直接表达「没回本之前不许作废」这条约束。代价是它只限制「下一次」重开，不修正「这一次」对视界的乐观；两者叠加时，一次乐观的重开最多让下一个阶段被拖长，不会再被连环作废。

为什么不保留固定间隔（例如 8 轮）。固定值与回本轮数无关：回本 4 轮的阶段被白白拖住，回本 15 轮的阶段照样提前作废。

为什么立即回本不受承诺限制。立即回本意味着重开那一轮就比不重开便宜，之后每轮还更便宜；上一个阶段还没回本的写已经是沉没成本，为它放弃更便宜的一轮不划算。离线回放里没有出现立即回本，校准重锚造成的抖动没有触发它，所以没有为此另加「重锚那一轮不许立即回本」的规则。

为什么不把「写价为 0」当作未知、按 1.25 罚。那等于把核对过的价格改成罚项，也会改写用户在 providers.json 里明确写的值，清单还会把它误记成默认来源。目录直接写输入价和价格页一致，用户想表达别的写价就写别的数。

为什么先验是比例。固定 token 数在小视图上会压过全部节省，在大视图上又几乎不起作用；比例和甩掉的量同尺度。

为什么不把 `expand` 取回的原文从每轮节省里扣掉。那样更接近真实，但要同时改两项，回本轮数的口径会变；本次只算一次性写入。

## Consequences

- 一个按回本开的阶段至少活过它承诺的轮数，期间只有立即回本与安全阀能结束它。
- 没有 `expand` 观测的会话，每次重开的回本轮数多出 `0.1 × 写价 ÷ 读价` 轮。
- 内置目录里自动缓存型号的写价倍率仍是 1.0，只是来源从 `max` 兜底变成目录真值。旧录制的 `request_header.pricing` 里写价仍是 0，回放按 `max` 得到同样的 1.0。
- `issue-133` 回放回归改为逐轮比对影子 Log 清单的 `oldTokens`、`newTokens`、`remainingTurns`、`readRate`、`rebuild`，并带上录制时的 system-reminder 申领单；以后改成本常数不必动它。
- 「不等式回本就开新阶段」用例的回本轮数随先验从 4 变 6。
- 子任务清单的 `epochDecision` 也带齐新字段：来源记 `default`，代价、已过轮数、承诺记 0，先验 share 记 0。
- 没加依赖。

## Testing

- `packages/plugins/test/compiler/rebuild-cost.test.ts`（改动前全红）：定价缺失才用默认值并标 `default`；用户写的 0 写价按输入价计、来源仍是 `pricing`；写价高于读价时额外成本更大、回本更晚，并把一次能回本的重开翻成不重开；阶段没活到承诺的回本时挡住按回本的重开，到了就放行；没有承诺不挡；立即回本不挡；安全阀照常；安全阀开的阶段承诺记 0；有 / 无 `expand` 观测的估算与差异、取回量封顶；`expandResultTokens` 只数成功的结果；`epochCommitment` 只读阶段第一份清单；编译器把这两个数从 Log 抄到清单上。
- `packages/plugins/test/replay/offline.test.ts`：回放 issue-63 录制会话，影子 Log 每份清单的 `epochDecision` 字段齐全。
- `packages/plugins/test/regressions/issue-133.test.ts`：见上，逐字段比对通过。
- `packages/provider/test/prepare.test.ts` 与 `decide.test.ts` 的 DeepSeek 用例按目录真值更新，回到写价 1.0、回本 17 轮的原始预期。
- 离线回放：仓库外一次性脚本对仓库里的三份回放 fixture 与一批私有长会话 Log 跑 `replayOffline`，比较改动前后的重开次数。fixture 前后都不重开；私有会话里有重开的会话多数持平或减少，没有会话变多，阶段承诺推迟了一部分相隔过近的重开，没有出现立即回本。另在私有录制上对账了重开后几轮的 `cache.expectedReadTokens` 与 `measuredReadTokens`：重开后几轮的实测缓存读明显低于预期，命中率比平时差一截，说明节省兑现得比不等式假设的慢。
- `pnpm check` 全绿。
