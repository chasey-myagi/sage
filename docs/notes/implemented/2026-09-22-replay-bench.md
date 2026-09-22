# 离线回放对照与真机基准

## Problem

策略换了之后，录好的 Log 上说不出省了多少、前缀还稳不稳、省略有没有伤到后面的引用。真机上又没有一套固定任务能把 resolved、空补丁和报错分开，也没有地方把「每个解决样本的 token、缓存命中率、expand 率」并排打出来。这两件事如果塞进同一次模型调用里，离线测试就得打真网。

## Decision

离线与真机分成两个接口。`replayOffline` 对每一轮只用该轮视图提交前的 path 前缀重编译，不调用模型。`recordedTokens` 取该轮 assistant usage 的输入合计。passthrough 的 `policyTokens` 等于这个数，`saved` 为 0。编译器策略的 `policyTokens` 是 `budget.used × calibration.ratio`，没有校准则倍率为 1。`prefixStable` 看相邻两轮重编译结果是不是前缀延长。`likelyNeededOmissions` 是被省略的工具块里，`toolUseId` 或 `accesses.key` 出现在后续 assistant 文本或 tool_use 输入中的块 id。模型打分器换成 `ruleScorer`，报表写 `scorer:offline-rule`。

`sage bench run` 在任务集上真跑。无 key 退出 2。每个任务 clone、checkout、跑 sage、再跑测试命令，记 `resolved | empty-patch | error`，连同 `runHealth` 写入 `<out>/results.json`，session 复制到 `<out>/sessions/<taskId>.jsonl`。`sage bench report` 打印两份结果的三率和三个数，以及 B−A。任务集是 Hugging Face 上 SWE-bench Verified 的前 50 条，由零依赖脚本拉取。基准不进 CI，本票不真跑。

报表只含数字、id 和 reason。Log 不存上下文窗口和定价，离线回放用 8000 和 `pricing: null`。

## Alternatives considered

为什么离线不打模型。新视图上的下一步动作只有模型才知道，录好的响应是旧视图的答案。拿旧响应去比新视图，一致率没有含义。离线能算的是确定性的三样：token、前缀、后续引用。动作是否一致、任务成不成，留给真机两套策略各跑一遍。

为什么疑似误删用「后续引用」做代理。被省略的块有没有被真正用到，离线看不到模型的注意力。后续 assistant 的文本或 tool_use 输入里出现该块的 `toolUseId` 或 `accesses.key`，说明这条轨迹后来还碰到了它。这是代理，不是漏判的证明。expand 率仍由真机清单上的 `expands/omitted` 给出。

为什么基准不进 CI。50 道题要 clone、打模型、跑测试，时间和费用都不适合 PR 门。任务集和脚本进仓库，触发留给手动。官方 docker 评测环境也不做：任务文件只提供 repo、commit、问题和测试命令，解释器和依赖由跑的人准备。

## Consequences

离线数字不能用来宣称成功率。编译器策略在影子 Log 上冻结自己的清单，所以对照的是「这套策略从头跑」而不是「在旧清单上打补丁」。`compareOffline` 的打分器标注挂在 `replayOffline` 返回的数组上，拷贝成普通数组之后标注就没了。`timeoutSec` 同时限制 sage 和测试命令。`bench/runs/` 是本地输出，不提交。

## Testing

红灯：`packages/plugins/test/replay/offline.test.ts` 首先因 `Cannot find module '../../src/replay/offline.ts'` 失败。`apps/sage/test/bench.test.ts` 在子命令接上之前，`bench run` 无 key 的退出码是 1，断言期望 2。

绿灯覆盖：已有 `text-only` 与 `tool-roundtrip` 在 passthrough 下 `saved` 为 0、`prefixStable` 为 true、`recordedTokens` 为 `[4]` 与 `[4, 7]`。`text-only` 在预算 8000 下 `policyTokens` 为 2、`saved` 为 2。人造会话里，预算 0 且 `keepRecent` 0 时，后续文本引用 `src/app.ts` 的块进入 `likelyNeededOmissions`，只引用了别的路径的块不进入；后续 tool_use 输入引用 `call_read` 时同样进入。`compareOffline` 的 Markdown 与 JSON 不含工具结果正文和用户原文。名为 `model` 的打分器调用次数为 0，报表含 `scorer:offline-rule`。

`bench run` 无 key 退出 2 且不写 `results.json`。注入的 clone / sage / diff / 测试把三条任务记成 resolved、empty-patch、error，`health.tokens.total` 为 6，结果文件和 session 都不含问题陈述。`bench report` 对夹具算出三率、`tokensPerResolved`、平均缓存命中率、平均 expand 率，以及 B−A。`fetch-swe-verified.mjs --fixture` 写出 pytest 命令，`--limit 1` 只留第一行。
