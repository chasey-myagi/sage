# 路由理由走结构化字段

## Problem

内核写 `model_change.reason` 时，`session.ts` 的 `routeReason` 从插件拼好的 `manifest.epochDecision.reason` 里用 `lastIndexOf(";route:")` 与 `startsWith("route:")` 切出路由理由。内核因此认识插件的 reason 拼法：插件改一下分隔符、在路由之后再追加一段后缀，或者路由策略自己的理由里带 `;route:`，`model_change` 的内容就跟着变，而内核这边没有任何类型提示。这违反「状态不编码进字符串」与「core 不认识 plugin」。

## Decision

core 新增导出类型 `EpochStart`，`CompiledView.epochStart` 用它：

```ts
type EpochStart =
  | { reason: string; model?: undefined; routeReason?: undefined }
  | { reason: string; model: ModelRef; routeReason: string };
```

`model` 与 `routeReason` 必须同时出现。内核在换模型时把 `routeReason` 逐字写进 `model_change.reason`，不解析、不拼接，删掉 `routeReason()`。

插件的 `openEpoch` 在路由返回模型时算一次 `route:<policy reason>`，同一个字符串既填 `epochStart.routeReason`，又作为后缀追加到 `epochDecision.reason`。两处出自同一个值，不会漂移。

写进 Log 的字节不变：`model_change.reason` 仍是 `route:<policy reason>`，`epochDecision.reason` 仍以 `;route:<policy reason>` 结尾。

## Alternatives considered

把 `model` 与理由合成 `epochStart.route: { model, reason }`：形状更紧，但 plugins 的路由测试断言的是 `view.epochStart?.model`，换位置就要改断言，这次只做内核去字符串协议，不动断言面。

`routeReason` 只放策略原话，由内核补 `route:` 前缀：内核仍然拥有一段插件的格式知识，只是从切分变成拼接。

`routeReason` 设为独立可选字段、缺省时内核写 `route:`：会留下「有模型没理由」这种合法状态，内核还得保留一个兜底字符串。判别联合让编译器拦住这种情况。

## Consequences

其他 ViewProvider 若在 `epochStart` 上给 `model`，必须同时给 `routeReason`，否则类型不过。core 的 `model-change.test.ts` 手写视图因此补上 `routeReason`（只改输入，断言未动）。`packages/core/README.md` 的 model_change 一节同步改写。

`epochDecision.reason` 仍是给人读、记 Log 的串，内核不再读它。

## Testing

`pnpm check` 全绿（89 个测试文件、468 个测试）。`core/test/session/model-change.test.ts` 对 `model_change` payload 的 `toEqual` 断言原样通过。另用一次性探针在 `apps/sage` 的真实编译器加路由阶梯场景下，分别在改动前后导出 `model_change` 与 `epochDecision` 的 JSON，两份逐字节相同：`"reason":"route:cheapest-allowed:openai/gpt-5.4-mini"`。探针未入库。
