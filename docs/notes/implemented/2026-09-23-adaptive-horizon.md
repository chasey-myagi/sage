# 自适应重建视界

## Problem

`remainingTurns` 是 `max(1, expectedTurns − 已开始轮数)`，`expectedTurns` 固定 20。2026-09-23 deepseek-flash、编译预算 8000 的 vue-10874 轨迹跑了 36 轮。turn 20 起剩余被夹成 1。清单从 `payback:37>1` 到 `payback:106>13`，输入超支到 17825 也不重开。可信版 bench 里还有 4 条任务跑到 41–50 轮。长任务的后半段，不等式右边永远是 1，回本要几十轮的重建通不过。

## Decision

`remainingTurns = max(1, expectedTurns − started, ceil(alpha × started))`。`started` 是 `max(0, turn − 1)`。`expectedTurns` 与 `alpha` 是 `contextCompiler` 的选项，默认 20 与 0.5。跑得越久，假定后面还有大约一半那么多轮。清单 `epochDecision.remainingTurns` 记下这个数，内核要求它是有限数。不等式、淘汰、冻结、校准不动。子任务不调用 `decideEpoch`，那条清单的 `remainingTurns` 仍是 1。

## Alternatives considered

为什么不用固定 20。固定 20 在 turn 20 把剩余收成 1。vue-10874 从 turn 8 起回本就要 36–106 轮，夹到 1 之后每一轮都是 `payback:N>1`，阶段永远不重开，过时的冻结块一直占着前缀。视界应该跟着已经观察到的任务长度走，而不是停在一个写死的轮数上。

为什么不用「无限」。剩余设成无限，任何正节省都会满足 `payback <= remaining`，长任务几乎每轮重开。重开改写前缀，缓存从第一个被改的 token 起全部作废。两个速度里前缀要慢。无限视界把「还打算走多少轮」从不等式里拿掉，等于只要省下的比例超过 `minSavingsRatio` 就砸前缀。

## Consequences

前 14 轮（`started ≤ 13`）`expectedTurns − started` 仍不小于 `ceil(0.5 × started)`，视界与固定 20 相同。从 turn 15 起下界抬起来：turn 30 是 15，turn 36 是 18。DeepSeek 读价倍率 0.02，这条 36 轮轨迹的回本仍是 36–106，每一轮都大于剩余，`rebuild` 仍全是 false。这是定价的结果。`alpha` 只在主编译路径上。离线回放不另传 `alpha`，用默认 0.5。#46 与 #50 的断言没有改。

## Testing

红灯时 turn 30 的 `remainingTurns` 是 1，期望 15；`alpha: 1`、turn 36 收到 1，期望 35。绿灯后同一条轨迹 turn 30 为 15、turn 36 为 18，36 轮的 `remainingTurns` 都是有限数，没有一轮 `rebuild`。`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、334 个测试）。`pnpm test:update` 没有改 `expected/`，也没有改 #46 / #50 的断言。它重写了 core 回放的 `session.jsonl`，只换了 id 和时间戳，已还原。
