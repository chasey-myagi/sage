# Jev 打分器

## Problem

模型打分要延长上一轮主请求才能吃到缓存，历史一长，辅助调用就不便宜。阶段重开时往往想给剩下的 kept 块全部再判一次档，但用聊天模型做这件事，等于把整段前缀再付一遍。TypeSafe 的 Jev（System One）不生成文本，一次请求可以带很多个带类型的问题，输入 $0.042/MTok、输出免费。它适合「只看查询和一行摘要，判断下一步要不要原文」。

## Decision

`jevScorer` 实现 `Scorer`。请求不带历史。`state` 是 `{ query, chunks: [{ id, tool, summary, tokens }] }`，每块一个 `choice`（`full` / `none`）。`probabilities.full` ≥ `minConfidence`（默认 0.5）判 full，否则 none。reason 是 `scorer:jev:<p>`，p 保留两位。不写 summary。回答里没有的块不出现在 decisions 里，编译器因此保持 `kept`。

分批同时受两道限制：每批最多 `batchSize` 块（默认 40），以及 `query.length + Σ(tokens × 4)` 不超过 24000 字符。单块自己超预算也单独发出。没有 key、非 2xx、超时（默认 10000ms）、正文不是 answers 对象，都返回 `{ decisions: [], fallback }`，不抛。429 和 529 最多再试 3 次，等待 `500ms × 2^n`。各批 `usage.input_tokens` 加进返回 usage 的 `inputOther`，其余三项为 0。

编译器在 decision 带了非空 `reason` 时，把这段字符串写成清单上的块 reason；没写的仍是 `scorer:model`。`manifest.scorer` 的 calls / usage / fallback 记账不变：一次 `score()` 记 1 次调用，批内的 HTTP 次数不另计。

目录增加 `typesafe` / `jev-latest`（上下文 32000，输入 0.042，输出和缓存读写为 0）。`listModels("typesafe")` 可见。CLI `--provider typesafe` 报错，说明它不是聊天端点。`--scorer jev` 读 `TYPESAFE_API_KEY`，缺失则退出码 2，并点名该变量。

## Alternatives considered

为什么用 choice 而不是 noul。noul 给出的是一个 0–1 的「是否」，没有「full」这个选项自己的概率。分档要的是下一步需要原文的概率，再和 `minConfidence` 比。choice 的 `probabilities.full` 就是这个数；`none` 是另一头，不是把一个是非题硬折成两档。

为什么不带历史。模型打分必须延长已经暖过的前缀，换掉 head 或丢掉 tools，缓存从第一个 token 就断开。Jev 不是那条前缀上的续写，它只回答一组关于块摘要的问题。把上一轮消息塞进 state，付的是整段上下文的输入价，换不来缓存命中，也不改变「一行摘要够不够」这个判断所需的材料。

为什么不产出 summary。summary 档要的是模型写的一句笔记，视图里会标成「不是证据」。Jev 不生成文本，没有可以落进那一档的句子。强行要它写摘要，就把它用成了聊天模型。本票只在 full 和 none 之间选。

## Consequences

确定性淘汰的四条规则没改。打分器仍只在阶段重开、且还有 kept 工具块时跑。离线回放碰到名字不是 `rule` 的打分器，仍换成 `ruleScorer`。`likelyNeededOmissions` 用 Jev 复判「后来被引用是不是真需要」留到以后，本票不做。

`typesafe` 没有聊天适配器。路由候选来自已装配的 adapter，不会把它选进梯子。子任务编译不装这个打分器。

没加依赖。测试用 `fetchImpl` 替身，不打真网。

## Testing

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、291 个测试）。合入 `origin/main` 的第三方端点改动后复跑，仍是 0。覆盖分档与 `scorer:jev:<p>`、200 块至少两批、429 两次后成功、529 退避、三次重试仍失败、超时、无 key、非法 JSON、非 2xx、缺答案保持 kept、usage 求和，以及 `--scorer jev` 无 key 退出 2、`--provider typesafe` 拒绝。
