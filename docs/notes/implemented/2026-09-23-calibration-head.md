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

阶段决策的 old/new、窗口阀一律是 `head + used × ratio`，ratio 大于 1 也照乘。`cache.expectedReadTokens` 用上一轮清单自己的校准去乘那一轮的冻结量：`prev.head + prev.ratio × prev.budget.frozenTokens`。上一轮没有校准时 head 为 0、ratio 为 1。这个数等于上一轮实际发出去的冻结前缀估算，重开后本轮重锚的 head 不参与。`measuredReadTokens` 仍是上一轮的 `inputCacheRead`。差距大就是前缀被砸了。

## Alternatives considered

为什么不用纯倍率。纯倍率就是现在的 `measured / budget.used`。固定头不在 `used` 里，第一轮 3017/519 = 5.81，第二轮把 617 估成 3587，第三轮把 4237 估成 21927。头是加法不是乘法。把它乘进每一块，短预算上的误差会被后面的全量块放大，安全阀看到的是假超窗。

为什么不用最小二乘。头和倍率可以拿整段轨迹一起拟合，残差会更小。校准要在每一轮、只凭已经落盘的 usage 算出来，同一份 Log 必须得到同一个数。最小二乘要等后文，或者每轮重解一遍，早期轮次会被后面的样本改写，回放对不上当时的决定。第一轮减出来的头已经把系统提示和工具表吃进去了；后面的 ratio 只缩放块这一截，大于 1 也乘回去。

为什么不钳位。`min(ratio, 1)` 让估算系统性低于实测。8000 窗口下 turn 3 实测输入 7538，已经超过 7200；钳位后 old 停在 `head + used`（回放里是 6735），安全阀不响，漏报。去掉钳位后同一轮约 8493，阀响，和实测在阈值同一侧。ratio 大于 1 是计数器低于供应商，乘回去才逼近实测。回放不带申领单时 turn 3 的 ratio 到 1.46，乘回整段 `used` 会过 0.9 线；这是正确判定，不是误触。钳住它方向反了。

## Consequences

回放这条 18 轮轨迹时，turn 2 起 `oldTokens` 与该轮实测输入的相对误差不超过 15%，turn 2–5 的 ratio 不超过 2。安全阀按两条验收，不要求每一轮的 reason 与实测过线同真值：同一阶段已经因这道阀开过之后不再重复重开。(a) 无误报——reason 含 `context-window-guard` 的每一轮，实测输入大于 0.9 × 窗口。(b) 不漏报——每个阶段里第一个 old 估算过线的轮次，reason 含该阀。8000 窗口回放里这两条都在 turn 3 成立；其余轮次 reason 不带这道阀，(a) 空真。

`cache.expectedReadTokens` 从 turn 3 起与 `measuredReadTokens` 的相对误差不超过 15%。turn 4 用上一轮清单自己的 head 与 ratio 去乘那一轮的 `frozenTokens`，得到 3193，对实测缓存读 3072，误差 3.9%。#46 那条 21 轮轨迹的安全阀仍开在 turn 4；`newTokens < oldTokens` 仍从第 8 轮出现，回本轮数长过剩余轮数，不再为 payback 重开。

没改淘汰、冻结和 `decideEpoch` 的公式，只改了送进去的 token，以及缓存期望读的是哪一轮的校准。没加依赖。回放期望文件没有改。
