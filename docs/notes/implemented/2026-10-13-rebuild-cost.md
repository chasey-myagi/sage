# 重建成本用实测费率、计入重建后代价、限制重建频率

## Problem

真实运行里一个任务会重开好几个阶段，每次重开都按全价重编前缀，缓存命中率下降，总成本可能高过不压缩。成本不等式有三处估得偏乐观：

- 写价。定价里 `cacheWritePerMTok` 为 0 时，`ratesFor` 把写价当成输入价（1.0）。0 既可能是「没有附加费」，也可能是「没公布」；这类 OpenAI 兼容端点的 usage 也不报缓存创建量，没有任何来源能证实 1.0。清单上也只有一个 `;pricing:fallback` 后缀，看不出读价、写价各自从哪来。
- 重建后代价。不等式只算省下的输入，没算压缩之后模型用 `expand` 把原文拿回来的那部分。
- 频率。只要回本轮数落在视界里就重开，两次重开之间可以只隔几轮，上一次的写还没回本就被扔掉。

## Decision

不等式变成：

```text
(旧 − 新) × 读价 × 剩余轮数  ＞  新 × 写价 − 旧 × 读价 + 预计取回 × 写价
```

- **费率与来源。**`ratesFor` 的输入是这次调用的定价，也就是本轮 `request_header.pricing` 记下的那一份。读价 `cacheRead / input`；写价在定价列了写价时是 `max(cacheWrite, input) / input`。定价缺失或输入价为 0 时读价、写价都用默认值 `DEFAULT_READ_RATE`（0.1）与 `DEFAULT_WRITE_RATE`（1.25）；`cacheWritePerMTok` 为 0 时读价照用定价，写价用 1.25。1.25 是内置目录里最高的写价倍率；写价往高估只会让重开更少。读价默认值不往高取，因为读价越低重开越少。`Rates` 的单一 `source` 拆成 `readSource` / `writeSource`，清单 `epochDecision` 新增 `readRateSource`、`writeRateSource`，取值 `pricing` 或 `default`。确知没有写附加费的端点，在 `~/.sage/providers.json` 把 `cacheWritePerMTok` 写成等于 `inputPerMTok`，写价就按定价算成 1.0。
- **重建后代价。**`postRebuildCost = 预计取回 × 写价`，计入 `extraCost`。预计取回不超过 旧 − 新。`log-read.ts` 的 `expandResultTokens` 从 path 读出每次成功 `expand` 的结果 token；有观测时 `expandEstimateFor` 给出 `{source: "observed", expandsPerEpoch, tokensPerExpand}`，预计取回 = 每阶段次数 × 平均 token。还没有观测时用先验 `{source: "prior", share: PRIOR_EXPAND_SHARE}`，预计取回 = 0.1 × (旧 − 新)。先验用比例而不是固定 token 数，让它在任何视图规模下都和甩掉的量成比例；它让回本轮数多出 `0.1 × 写价 ÷ 读价` 轮。清单记下 `postRebuildCost` 和 `expandEstimate`。
- **最小间隔。**`DEFAULT_MIN_REBUILD_INTERVAL` = 8，`contextCompiler({ minRebuildInterval })` 可改。阶段从第一份清单的 turn 算起不满 8 轮时，不等式批准了的重开（包括 immediate）也改成不重开，原因 `min-interval:<已过轮数><<间隔>`，trigger 是 `no-rebuild`。上下文窗口安全阀排在这条之前，不受限。清单记下 `turnsSinceEpochOpen` 与 `minRebuildInterval`。
- core 的 `EpochDecisionReport` 同步新字段；`kinds.ts` 的校验不变（仍只要求 `remainingTurns` 有限），Log entry 的 kind 文档不变，`docs/log-manifest.md` 无需重生成。

## Alternatives considered

为什么写价为 0 时不继续当 1.0。上一版（[deepseek-tuning](./2026-09-23-deepseek-tuning.md)）把 0 读作「没有附加费、首次按未命中价付」，这对 DeepSeek 与 OpenAI 的价格页成立，但对用户在 providers.json 里没填写价、或网关不公布写价的端点不成立，而 sage 从 `Pricing` 里分不出这两种。估低写价会让一次不划算的重开看起来划算，估高只会少开几次阶段，后者可以被安全阀兜住。确知 1.0 的人有 providers.json 这条显式路径。改内置目录、给 OpenAI 等写成 `cacheWrite = input` 也能区分，但那会让目录里的价格和价格页上的数字不一致，目录是按价格页逐项核对的。

为什么先验是比例而不是「每阶段 1 次、每次 1000 token」。固定 token 数在小视图上会压过全部节省：几十 token 的玩具视图永远不重开，几十万 token 的视图里又几乎不起作用。比例先验和甩掉的量同尺度。

为什么有观测时不和先验混合。混合要再引入一个权重常量，而 issue 要的是「有观测用观测，没有用先验」。观测量的上限是甩掉的量，单次巨大的 `expand` 不会让预计取回超过这次重建真正甩掉的部分。

为什么不把 `expand` 拿回的原文也从每轮节省里扣掉。那样更接近真实（取回的块之后每轮都要读），但要同时改每轮节省和额外成本两项，回本轮数的口径会跟着变；本次只把它算成一次性写入，口径保持「每轮节省 × 轮数 ＞ 一次性代价」。

为什么间隔也挡住 immediate。issue 只把安全阀列为例外。immediate 意味着重开那一轮就更便宜，挡住它会少省几轮，但 immediate 往往出现在估算抖动最大的时候（校准刚重锚、视图刚变小），统一规则更容易推理。

为什么间隔取 8。按回本算，一次重开通常要好几轮才把自己的写赚回来，8 轮与默认参数下常见的回本轮数同一量级；离线回放里改动前出现过相隔只有几轮的两次重开，8 能把这种情况推迟到上一次大致回本之后，同时在长会话里仍允许多次重开。

## Consequences

- 内置目录里写价为 0 的型号（OpenAI、DeepSeek、Kimi、百炼的自动缓存档）写价倍率从 1.0 变成 1.25，回本轮数变长，重开变少。Anthropic 的写价来自定价，不变。
- 没有 `expand` 观测的会话，每次重开的回本轮数多出 `0.1 × 写价 ÷ 读价` 轮。
- 阶段开头 8 轮内只有安全阀能重开。依赖「阶段第二轮就按成本重开」的测试改为显式传 `minRebuildInterval: 0`，见 Testing。
- `issue-133` 回放回归原来逐字比对 reason；reason 里的回本轮数随成本模型变化，录制时的模型不能再复现，改为比对 `rebuild` 和 reason 的规则名（`:` / `;` 之前的部分）。
- 子任务清单的 `epochDecision` 也带齐新字段：来源记 `default`，代价与间隔记 0，先验 share 记 0。
- 没加依赖。

## Testing

- 新增 `packages/plugins/test/compiler/rebuild-cost.test.ts`，改动前全部红灯：定价缺失退回默认值且两个来源都是 `default`；写价为 0 时读价照用定价、写价用默认值；写价高于读价时 `extraCost` 更大、回本更晚，并把一次能回本的重开翻成不重开；最小间隔挡住 payback 与 immediate，安全阀照常触发；无观测用先验 share，有观测按次数与平均 token 估，且不超过甩掉的量；预计取回能把一次临界重开翻成不重开；`expandResultTokens` 只数成功的 `expand` 结果；`turnsSinceEpochOpen` 按阶段第一份清单的 turn 计。
- `packages/plugins/test/replay/offline.test.ts` 新增回放用例：回放 issue-63 的 deepseek-flash 录制会话，影子 Log 里每份清单的 `epochDecision` 字段齐全，`readRateSource` 为 `pricing`、`writeRateSource` 为 `default`。
- 改动的旧用例：`decide.test.ts` 的费率断言换成新形状；DeepSeek 视界用例在写价 1.25 下回本 22 轮，视界改成 25 与 10 两档；「不等式回本就开新阶段」用例显式 `minRebuildInterval: 0`，回本轮数随先验从 4 变 6。`apply.test.ts`、`expand.test.ts` 两个钉住重开后视图形状的用例显式 `minRebuildInterval: 0`。
- 离线回放：用仓库外的一次性脚本，对仓库里的三份回放 fixture 与一批私有长会话 Log 分别在改动前后跑 `replayOffline`，比较 `rebuild` 次数。fixture 改动前后都不重开；私有会话里多数有重开的会话重开次数下降，总数下降几成，没有会话变多。消融显示下降主要来自写价默认值，先验贡献很少；间隔在这批数据上只推迟了少数相隔过近的重开，没有改变总数。
- `pnpm check` 全绿。
