# 阶段内决策累积冻结

## Problem

2026-09-23 deepseek-flash 的 21 轮长会话里，每轮 `epochDecision.oldTokens === newTokens`，原因全是 `savings-too-small`，阶段重开没有发生。同一阶段内，已经判过档的块有 11 次从 full 翻成 none。翻档那一轮缓存读骤降、全价输入飙升：turn 7→8 缓存读 10752→3712、全价 424→5546；turn 11 缓存读 3200、全价 9523；turn 16 缓存读 3456、全价 7192。整段命中率 76%，翻档轮拖了后腿。

`frozenDecisions` 只读阶段第一份 manifest。之后新出现的块每轮重新进入 `eliminate`，`superseded_by` 会把更早、但不在那份清单里的块改判为 none。前缀中段被改写，缓存从第一个被改的 token 起全部作废。当前视图和「全量重编译」又都已经做了淘汰，不等式没有差值。

## Decision

冻结按阶段累积。自该 epoch 的 `view_epoch` 起（阶段 0 没有 `view_epoch` 时从第一份清单起到下一个 `view_epoch`），按出现顺序合并块的 id → tier，后出现的不覆盖先出现的。没有 manifest 时仍是 undefined。

已在 map 里的块原样沿用 tier 和 reason，桩文本由同一档位重新渲染，字节不变。`eliminate` 的输入是这些已冻结块（token 记 0，只作 superseded / recent 的参考）加上本轮第一次出现的块。只有新块采用淘汰结果。已冻结且非 none 的 token 先从剩余预算里扣掉，oldest-first 只能再降新块，降完的档位同样冻结。新块在历史末尾，不会被更早的块盖掉，所以首轮是 `recent` 或 `full`，除非预算把新块降成 none。

`oldTokens` 仍是冻结视图的 `used` 乘校准倍率（上一轮 usage 实测 / 当时 `budget.used`，没有校准则是 1）。`newTokens` 是同一轮对全部块不带冻结再跑一遍淘汰、加上申领单之后的 `used`，乘同一个倍率。重开时选用这份全量重编译，新阶段从这份清单重新冻结。

清单 `budget` 增加 `frozenTokens`（冻结视图里非 none 块的 tokens 合计）和 `staleOmittable`（其中全量重编译会改判为 none 的部分）。重开当轮这两个数仍按重开前的冻结视图计算。子任务重编译不比较重开，两者为 0。

`rebuildFromLog` 在 `view_epoch.details.epochHead` 已经是投影历史里的某条消息时不再把它插到前面。编译器本来就这样跳过「本身就是 Log 里的历史」的阶段头；重开一旦发生，不跳过就会把同一条 user 消息写两遍，L4 对不上。

`bench run` 在没写 `--provider` 时，按与 `sage -p` 相同的预设顺序认 key，不再只看 `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`。

## Alternatives considered

为什么不允许 `superseded` 改已冻结的块。覆盖读确实能把更早的 tool_result 换成一行桩，单看正文是省 token。真机翻档的代价不是那一行：turn 7→8 缓存读从 10752 掉到 3712，全价输入从 424 升到 5546。前缀改一个字节，后面整段按未命中重读。省下的桩和这笔缓存作废不是一个数量级。过时的块留到下一次被不等式批准的重开再清，是「两个速度」里前缀慢、尾部快的取舍。

为什么不把冻结记成一种新 entry。档位和 reason 已经在清单上。再加一种 entry，内核就得认识插件的「冻结」，也和「清单落在不透明 payload」重复。累积合并仍只读 manifest。

为什么 `newTokens` 不是把块的 `used` 乘倍率再单独加申领单。校准倍率来自上一轮整份 `budget.used`（申领单和块在一起）。只放大块、申领单不放大，旧视图和新视图就不是同一口径，比值不是 1 时会造出假差值。两边都是「申领单 + 块」的 `used` 乘同一个倍率。

## Consequences

阶段内已判定块的档位不再变，`renderLlmInput` 在未重开的轮次是 `<last>` 加增量。重开那一轮允许整段重写：新视图应用全部淘汰（覆盖读变成 none），下一轮起按新阶段冻结。

用留档的 21 轮轨迹做 `replayOffline`（budget 40000、keepRecent 6、expectedTurns 20；回放窗口仍是 8000、定价 null）：已判定块中途翻档 0 次；从 turn 8 起 `newTokens < oldTokens`；turn 12 为 `payback:9<=9` 重开。turn 3 和 turn 13 仍是 `context-window-guard`，因为校准后的估算超过 8000 的 0.9。回放期望文件没有改。

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、301 个测试）。没加依赖，没改淘汰规则和打分器。
