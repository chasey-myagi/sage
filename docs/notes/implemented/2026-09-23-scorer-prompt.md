# 同模型打分器：说清任务、禁止调工具、解析器容忍

## Problem

Issue #69 记录 DeepSeek 上 249 次影子调用，标题概括为 249/249 次失败：236 次 `invalid-json`，12 次 `timeout`，1 次成功，判定分布为空，对账数全 0。用 tee 过的 adapter 复现 4 次，原始输出是两种。2 次 `finish=tool_use`，模型直接调 `edit` 继续干任务，thinking 里写着最终指令要求只输出 JSON。2 次 `finish=stop`，输出单个对象 `{"tier":"summary","summary":"Read … appended (reviewed) …"}`，在总结自己刚做的事，不是逐块判定。

## Decision

指令改成旁路判定：说明每个候选是早先某次工具调用的结果，输出是 JSON 数组，每项 `{id, tier, summary?}`，给一个两项示例，并写上 `do not call tools`。`Request` 增加可选 `toolChoice?: "auto" | "none"`。打分请求设 `"none"`。openai-chat 映射成 `tool_choice: "none"`，anthropic-messages 映射成 `{ type: "none" }`。没设这个字段时请求体不带它。head、tools 和 messages 前缀逐字节不变。

解析器去掉一层 json 围栏。顶层单个对象带 `id` 时包成数组。`id` 不在候选清单里的项丢弃，个数记在 `score()` 的 `dropped` 上。空数组是全部保留，不是失败。缺 `id` 或 tier 非法则整段 `invalid-json`。fallback 且已经拿到模型回复时，`raw` 为原文前 300 字加 finish 种类，编译器只把它透传到 `manifest.scorer.raw` 和 `manifest.shadow.raw`。成功不写 `raw`。

## Alternatives considered

不去掉 tools。打分请求要接在已经暖过的前缀上，少一个 tool 或换掉 system，缓存从第一个 token 断开，整段历史按全价再付一遍。`tool_choice: none` 只禁止这一次调用工具，不改 tools 和 messages 的字节。

不保留旧的 `{decisions:[...]}` 包裹。真机吐的是数组或单个对象，预发布只留一种形状。`dropped` 不进清单：简报只让编译器透传 `raw`，丢弃计数留在这次 `score()` 的返回值上。

## Consequences

确定性淘汰、冻结、阶段决策和校准没改。`apps/**` 和 `packages/tools` 没动。没加依赖。jev 的解析仍是 answers 对象，没有抽共用解析器。没装打分器的清单不带 `scorer.raw`。影子成功时也不带 `shadow.raw`。

真机复现脚本由作者持有，合入前后各跑一次，不进 CI。测试不打真网。

## Testing

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、361 个测试）。覆盖 json 围栏数组、单对象包成一项、无 id 对象 `invalid-json` 且 `raw` 含原文前缀与 `finish: stop`、空数组不失败、未知 id 丢弃并计数、原文截到 300 字、指令快照含 `seq:1#a` 与 `do not call tools`、openai `tool_choice: "none"` 与 anthropic `{type:"none"}` 且 messages 前缀和 tools 与上一请求相同、默认请求不发该字段、`manifest.shadow.raw` 只在 fallback 时出现。
