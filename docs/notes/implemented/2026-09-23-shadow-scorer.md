# 影子打分

## Problem

在 DeepSeek 上打分器从没跑过：它只在阶段重建那一轮被调用，而重建几乎永不回本。20 条会话 scorerCalls 全 0。rule / model / jev 准不准没有样本。要评估，得先让打分器每轮在旁边判一次，只写 Log，任务结束后用「这块后来有没有被 expand 取回，或被同一路径再次 read」当弱标签对账。

## Decision

`contextCompiler({ shadowScorer })` 在档位、冻结和预算定下来之后，对本轮新进入视图的工具块调用 `shadowScorer.score`。新块是上一份清单里没有的 chunk id；第一轮全部工具块算新的。`prevRequest` 用这一轮编译完的视图。结果只写 `manifest.shadow`，不改档次、冻结、预算和模型可见消息。抛错只记 `fallback`。

`sage --shadow-scorer rule|model|jev` 默认关闭。`bench run` 把这个旗标透传给每个任务。`runHealth` 增加 `shadow: { calls, usage }`。`sage stats` 在健康度旁边打印 `shadowNone`、`laterNeeded`、`precision = 1 − laterNeeded / shadowNone`（没有 none 时 precision 为 null）。后来仍需要：块 id 出现在更后面的 `expand` 参数里，或该块是 read 结果且同一路径在后续轮次被再次 read。

## Alternatives considered

为什么不把影子档写回清单的 tier。写回去就变成又一个会改前缀的打分器，20 条会话上仍然没有「只观察、不干预」的对照。评估要的是同一条轨迹上的标签，不是再开一轮会改冻结的实验。

为什么不用上一轮请求当 prevRequest。正常打分要延长已经发出去的前缀。影子发生在本轮编译完成之后，喂给它的是这一轮即将发出的视图，辅助调用才能接在当前前缀上。子任务编译不跑影子，避免把父前缀再打一遍分。

为什么 same-turn 的再次 read 不算后来仍需要。后续轮次指的是下一次及更后的编译轮，不是这一轮模型在判完之后立刻又读了一次。同一轮里的第二次 read 还没跨过「下一轮仍要」这条线。

## Consequences

确定性淘汰、冻结、阶段决策和校准没改。原来的 `scorer` 仍只在重建时跑。没装 `shadowScorer` 的清单不带 `shadow`，健康度里 calls 和 usage 是 0。离线回放不调用影子打分器。

没加依赖。对账用 vue-10874 的真机轨迹加合成 expand，测试不打真网。`bench run --shadow-scorer model --provider deepseek` 留到有 key 时再跑。

## Testing

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、347 个测试）。覆盖五轮新块只进 `shadow.decisions` 且档位与无影子逐字节相同、打分器抛错只记 `fallback`、stats 对账 `shadowNone = 5` / `laterNeeded = 2` / `precision = 0.6`。
