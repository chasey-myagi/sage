# 校准改成固定头加倍率

## Problem

2026-09-23 deepseek-flash 的 18 轮轨迹（`long-deepseek-flash-20260923-0239.jsonl`）里，`budget.used` 只算块和申领单，系统提示、工具 schema 和任务消息这约 2500 token 的固定头不在里面。`calibrate.ts` 用 `measured / estimated` 一个倍率去缩放，固定头被当成块的比例摊出去。

| turn | budget.used | 实测输入 | ratio | 用 ratio 估出的 old | cache.expected | cache.measured |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 519 | 3017 | – | 519 | 542 | 0 |
| 2 | 617 | 3193 | 5.81 | 3587 | 522 | 0 |
| 3 | 4237 | 7538 | 5.18 | 21927 | 522 | 2944 |
| 4 | 6855 | 10399 | 1.78 | 12196 | 522 | 3072 |
| 6 | 10224 | 11024 | 1.29 | 13159 | 522 | 10368 |
| 18 | 15419 | 16786 | 1.09 | 16779 | 522 | 16512 |

turn 3 把 4237 估成 21927，相对误差 190%。离线回放窗口是 8000，这一轮（这条轨迹上实际是 turn 2，估算 7240）撞了 `context-window-guard`。真机窗口大没撞，但 old/new 与 payback 前几轮全是错的数。`measured ≈ head + used` 则拟合得上：head = 3017 − 519 = 2498，turn 2 预测 3115（实 3193），turn 3 预测 6735（实 7538，差 12%）。

`cache.expectedReadTokens` 一直停在 522 左右（只覆盖了 epoch-head 申领），而缓存读的是整个冻结前缀。turn 18 的清单字段是上一轮的 16384，该轮 usage 的 `inputCacheRead` 是 16512。这个字段没法判断前缀稳不稳。

## Decision

`calibration` 改为 `{ measured, estimated, head, ratio }`。`head` 是本阶段第一轮实测输入减去该轮 `budget.used`，没有 usage 时用路径上最近一次算得出的值，小于 0 记 0。之后各轮 `ratio = (measured − head) / estimated`。缺数据时不写校准，调用方用 head 0、ratio 1。

阶段决策的 old/new 是 `head + used × min(ratio, 1)`。锚点轮的 ratio 正好是 1，所以 turn 2、turn 3 就是 `head + used`（回放里 turn 3 为 6735）。ratio 大于 1 不乘回整段 `used`：回放不带申领单，分母只有块，turn 3 的 ratio 会到 1.46，估出来约 8493，8000 窗口的 0.9 又被误触。ratio 小于 1 要乘回去，那是计数器比供应商偏高，不收回来的话后半段会高出 15%。

`cache.expectedReadTokens` 改为上一轮冻结前缀：`head + min(ratio, 1) × (budget.frozenTokens + epoch-head 申领 token)`。`measuredReadTokens` 仍是上一轮的 `inputCacheRead`。差距大就是前缀被砸了。

## Alternatives considered

为什么不用纯倍率。纯倍率就是现在的 `measured / budget.used`。固定头不在 `used` 里，第一轮 3017/519 = 5.81，第二轮把 617 估成 3587，第三轮把 4237 估成 21927。头是加法不是乘法。把它乘进每一块，短预算上的误差会被后面的全量块放大，安全阀看到的是假超窗。

为什么不用最小二乘。头和倍率可以拿整段轨迹一起拟合，残差会更小。校准要在每一轮、只凭已经落盘的 usage 算出来，同一份 Log 必须得到同一个数。最小二乘要等后文，或者每轮重解一遍，早期轮次会被后面的样本改写，回放对不上当时的决定。第一轮减出来的头已经把系统提示和工具表吃进去了；后面的 ratio 只记录块这一截还差多少，大于 1 的那截不拿去改决定。

## Consequences

回放这条 18 轮轨迹时，turn 2 起 `oldTokens` 与该轮实测输入的相对误差不超过 15%，turn 2–5 的 ratio 不超过 2，turn 3 不再是 `context-window-guard`。turn 4 的估算超过 8000 的 0.9，安全阀在这一轮打开，这是请求本身已经过线，不是头被倍率摊出来的假超窗。

`cache.expectedReadTokens` 从 turn 3 起与 `measuredReadTokens` 的相对误差不超过 15%。#46 那条 21 轮轨迹不再在 turn 3 误开安全阀，改为 turn 4；`newTokens < oldTokens` 仍从第 8 轮出现，但 head 进了新旧两边，回本轮数长过剩余轮数，不再为 payback 重开。

没改淘汰、冻结和 `decideEpoch` 的公式，只改了送进去的 token。没加依赖。`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、306 个测试）。回放期望文件没有改。
