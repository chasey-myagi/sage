# 第三方 OpenAI 兼容端点

## Problem

真机验收不打算走 Anthropic。OpenAI 兼容适配器的构造参数已经有 `baseUrl`，CLI 没暴露；DeepSeek 的缓存命中是 `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens`，现有映射只认 `prompt_tokens_details.cached_tokens`，命中会被算成 0。目录里没有 DeepSeek、百炼、Kimi 的现行型号。

## Decision

`resolvePreset` 是一张纯数据表：`openai`、`deepseek`、`dashscope`、`moonshot`、`anthropic`，各自带 base URL、环境变量名和 api。`--provider` 选预设，`--base-url` 与 `--api-key-env` 只覆盖这一次选中的预设。有 key 的预设各装一个适配器；四家 OpenAI 兼容端点是同一个 `OpenAIAdapter`，只换基址。选中的预设没有 key 时退出 2，stderr 点名那个环境变量。没写 `--provider`、也没有 Anthropic 或 OpenAI 的 key 时，仍退出 1。

usage 按字段优先级映射：有 `prompt_cache_hit_tokens` 则命中进 `inputCacheRead`，`prompt_cache_miss_tokens` 进 `inputOther`（缺 miss 时用 `prompt_tokens - hit`）；否则有 `prompt_tokens_details.cached_tokens` 则沿用原公式；否则整段 `prompt_tokens` 进 `inputOther`。`inputCacheCreation` 恒为 0。`raw` 是供应商 usage 对象，不改。

目录新增 DeepSeek（`deepseek-flash` 默认、`deepseek-v4-pro`）、Kimi（`kimi-k2.6` 默认、`kimi-k2.7-code`、`kimi-k3`）、百炼（`qwen-plus` 默认、`qwen-max`、`qwen-flash`）。`Pricing.currency` 可选，缺省 USD；百炼和 Kimi 标 `CNY`。`ratesFor` 不读币种。

## Alternatives considered

把 CNY 按汇率折成 USD 再入表。目录要的是官方页面上的单价和 source URL，汇率一变数字就和页面对不上，也说不清折的是哪一天。比率只是读价除以输入价、写价除以输入价，单位在除法里消掉，币种不参与。跨币种比绝对价是调用方的梯子问题，默认梯子只有当前型号，不会把人民币和美元放在一起选最便宜。

给这些端点填一个非 0 的 `inputCacheCreation`。DeepSeek、百炼兼容模式和 Kimi 的 usage 报文没有缓存写入计数；Kimi K3 的写费是显式上下文缓存的价目，不是这次响应里的 token 数。没有字段就记 0，避免把命中或未命中再算一遍写费。`raw` 留着，回放仍能看见原始字段。

## Consequences

兼容端点的缓存是服务端自动前缀缓存。请求体仍然不写 cache 字段，`cacheBreakpoints` 在 OpenAI 适配器上无操作。

DeepSeek 入表的是峰时价。非峰时减半；峰时是 UTC 工作日 01:00–04:00 与 06:00–10:00。百炼入表的是页面原价，促销可能更低；`qwen-plus` 思考模式输出价是 8，表内 2 是非思考输出。`qwen-flash` 的 0.15 / 1.5 是输入 ≤128K 的第一档。上下文：`qwen-plus` 与 `qwen-flash` 为 1,000,000，`qwen-max` 为 32,768。

2026-09-23 抓取的百炼价格页上，模型 ID `qwen-max` 的华北原价是输入 2.4 元、输出 9.6 元；12 / 36 对应的是 `qwen3.8-max`。本票按已批准简报把 `qwen-max` 写成 12 / 1.2 / 36，source 仍是该页。这是和页面不一致的一格，合并前可以改。

`costTierPolicy` 仍按 `inputPerMTok` 的裸数字比大小。把 CNY 型号和 USD 型号放进同一条 `--ladder` 时，比的不是同一种货币。

## Testing

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、vitest 275 项）。provider 测试用注入的 `fetchImpl` 回放三种 usage fixture，不打真网。CLI 测试把全局 `fetch` 换成固定 SSE，断言 `--provider deepseek` 的 URL 与 `Authorization`，无 key 时期望退出 2。
