# 面向 DeepSeek 的写价、推理与默认 provider

## Problem

`ratesFor` 把 `cacheWritePerMTok / inputPerMTok` 直接当写价倍率。DeepSeek、OpenAI 和多数兼容端点的缓存写附加费是 0，倍率就成了 0。重建额外成本变成 `0 − old × read`，不大于 0，于是每轮 `immediate` 重开，阶段前缀冻不住。

DeepSeek 推理流在 `delta.reasoning_content` 里给字符串增量。OpenAI 适配器丢掉这些增量，模型看见过的内容没有 entry，违反 L4。

CLI 在没写 `--provider` 时先看 Anthropic 再看 OpenAI。只有 `DEEPSEEK_API_KEY` 时进程退出 1。目录里 deepseek 的第一条是 `deepseek-flash`，不写 `--model` 就用它。

## Decision

`ratesFor` 的写价改为 `max(cacheWritePerMTok, inputPerMTok) / inputPerMTok`，读价仍是 `cacheRead / input`。定价缺失或输入价为 0 时仍退回 `{read: 0.1, write: 1.25, source: "fallback"}`。`decideEpoch` 的不等式不改。DeepSeek flash 的定价因此是 `{read: 0.02, write: 1}`。旧 150k、新 40k、剩余 20 轮时 `extraCost = 37000`，`paybackTurns = 17`，重开；剩余 10 轮时不重开。

OpenAI 适配器把 `delta.reasoning_content` 的字符串增量收成独立 index 的 thinking 块，发 `thinking_delta`，没有 signature。切到 `content` 或该 choice 结束时发 `block_end`。编码仍走已有的 `normalizeForApi`，不另写一条剥除路径，请求体不回传 `reasoning_content`。

没写 `--provider` 时，按 deepseek、dashscope、moonshot、openai、anthropic 取第一个已设 key 的预设。显式 `--provider` 盖住这个顺序。deepseek 目录把 `deepseek-v4-pro` 放到第一条，`deepseek-flash` 保留，用 `--model` 选。

## Alternatives considered

为什么写价倍率用 `max(cacheWrite, input)`，而不是给 DeepSeek 单独填 1.0。写附加费为 0 的不只 DeepSeek：OpenAI、百炼、Kimi 的自动前缀缓存同样不另收写费，第一次仍按未命中的输入价付。给一家写死 1.0，其余家继续是 0，`immediate` 重开还在。`max` 对 Anthropic 仍是 1.25（写价高于输入价），对写价为 0 或等于输入价的型号是 1.0，不用按 provider 分支。

为什么 reasoning 进 Log 但不回传。L4 要求模型看见过的内容能从 Log 重建，所以增量要落成 thinking 块。DeepSeek 要求下一轮不要把推理内容发回去；`openai-chat` 的 `normalizeForApi` 已经丢掉全部 thinking。再在编码里拼一个 `reasoning_content` 字段，会把不该回传的文本送出去，也重复了一条已经存在的规整。

## Consequences

凡是目录里 `cacheWritePerMTok` 为 0 的型号，写价倍率从 0 变成 1。阶段重开不再因为「写价免费」而每轮 immediate。Anthropic 的 1.25 不变。定价为 null 的回放仍用 0.1 与 1.25，期望产物没有改。

`cacheHitRatio` 仍是实测除以 `expectedReadTokens`，不把 64 token 的块对齐差抹平。README 写明：DeepSeek 只对完全相同的前缀、按 64 token 一块计费；人工比对时，相差不到一块是对齐，不是没命中。目录单价是峰时价。`decideEpoch` 只用比率，非峰时半价不改变开不开新阶段。`bench report` 不做按时段换算。

两家 key 都在时，openai 现在排在 anthropic 前面。只有 `DEEPSEEK_API_KEY` 时，`sage -p` 走 deepseek 和 `deepseek-v4-pro`。

没加依赖。测试用 fixture 与注入的 `fetchImpl`，不打真网。

## Testing

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、vitest 297 项）。红灯时 DeepSeek 写价倍率是 0，`extraCost` 为 -3000；`reasoning_content` 没有变成 `thinking_delta`；只有 `DEEPSEEK_API_KEY` 时退出码是 1；目录第一条仍是 `deepseek-flash`。

## 后续

内置目录里这些型号的 `cacheWritePerMTok` 已改为直接写输入价（价格页写的就是「未命中按输入价、无创建费」），写价倍率仍是 1.0，`max` 只兜住用户自填的 0。定价来源字段改名为 `rateSource`（`pricing` / `default`）。见 [rebuild-cost](./2026-10-13-rebuild-cost.md)。
