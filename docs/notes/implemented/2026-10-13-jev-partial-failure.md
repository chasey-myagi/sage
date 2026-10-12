# Agent Note: jev 部分请求失败只放弃失败的块

## Problem

jev 打分器每个候选块发一个请求，最多 4 个并发。任何一个请求失败（429/529 重试用尽、超时、非 2xx、正文不是 answers），`postRequests` 就停止派发新请求，整次结果变成 `{ decisions: [], fallback }`。编译器见到 `fallback` 丢弃全部判定：重建打分回到淘汰档位并在 `epochDecision.reason` 追加 `;scorer:fallback:<原因>`，入口打分把所有候选留在 full，影子只记 fallback。已经付费、已经拿到的回答一并作废。重建轮一次可能有几十个请求，碰上速率上限时整轮白费。

## Decision

- **失败只作用于自己的块**：一个请求失败后其余请求照发，成功的判定照常返回。失败的块不给判定，编译器沿用「回答里没有的 id 原样保留」：重建保留淘汰档位，入口保持 full，影子没有这条记录。
- **只有全部请求都失败才 fallback**：原因取第一个失败的请求，不带 usage（没有完成的请求），不带 `failedRequests`。fallback 的含义因此仍是「这次打分什么都没拿到」，`scorerFallbacks` 的口径不变。
- **部分失败用结构化计数上报**：`ScoreResult.failedRequests` 只在部分失败时出现，且大于 0。编译器经 `score-common.ts` 的 `failedRequestsField` 把它抄到 `manifest.scorer`、`manifest.entryScored`、`manifest.shadow` 上，三处同一个字段名、同一个口径；没有失败时不写这个字段，旧清单和回放夹具的形状不变。`dropped` 仍只表示解析器丢掉的项，两者不混用。
- **`signal` 中止后不再发新请求**：还没开始的请求直接按 `aborted` 计入失败，不打网络。若中止前已有请求答完，这次结果仍是部分成功。编译器目前不传 `signal`，只有直接调用 `score` 的调用方会传。
- **429/529 重试用尽后熔断**：任一请求在 429 或 529 上重试用尽，之后还没开始的请求走同一条「不发送、计为失败」的路径，原因取该请求的 `http:429` 或 `http:529`；已在途的照常结束，已答上的判定保留。熔断由 `http.ts` 失败结果上的结构化标记 `rateLimited` 触发，不读 `fallback` 文本。这是墙钟保护：速率上限下每个块各自重试 3 次，几十个块串起来要等很久。
- **kinds 校验**：三个报告上的 `failedRequests` 出现时必须是有限数。`scorer` 报告此前只校验 `raw`，现在也校验这个字段。
- **health 汇总**：`runHealth` 新增 `scorerFailedRequests`（重建），`entryScored.failedRequests`，`shadow.failedRequests`，缺字段的清单加 0。
- **bench report**：`BenchMetrics.scorerFailedRequests` 是各任务三项之和，是计数不是比率；没有任何任务带 health 时为 null，`delta` 照常相减。

## Alternatives considered

- **全部失败时也写 `failedRequests`**：数字更全，但 fallback 已经说明「一个都没答上」，再写计数是同一事实两个来源；还会改变所有现有 fallback 结果的形状。
- **复用 `dropped`**：`dropped` 是解析器丢弃的回答项，不进 Log；失败请求是传输层的事实，要进清单给 health 汇总。语义不同，不合并。
- **第一次 429 之后整体退避或停发**：能少打几次速率上限，但失败块之外的请求本来就会自己按 `retry-after` 重试；停发等于回到旧行为，issue 要的是保住已经能拿到的判定。熔断因此只在重试用尽后才打开：那时服务端已经连续拒绝了 4 次，继续发只会把墙钟拉长。
- **bench 里拆成三个指标**：多数任务只开一种打分方式，三个数分开对比价值不大；health 里已经分开，需要时可以从 `results.json` 读。

## Consequences

- 重建轮在速率上限下不再整轮回退，`epochDecision.reason` 也不再因为单个块失败追加 `scorer:fallback`。想知道这一轮有多少块没被打分，看 `manifest.scorer.failedRequests`。
- 部分失败时成功块与失败块的档位可能不一致（一部分被 jev 判成 none，一部分保持淘汰档位），这是有意的：失败块回到确定性的默认档位，和整次 fallback 时的行为相同，只是范围缩到了块。
- 部分失败时 jev 发出的请求数比以前多（非速率类失败后不再提前停止）。429/529 场景下，熔断打开前已在途的请求各自重试最多 3 次，之后的请求不再发出，所以速率受限时一次打分的墙钟仍有上界。

## Testing

- `packages/plugins/test/regressions/issue-181.test.ts`：用 `jsonFetch` 替身，6 个工具块，只让一个块的请求失败。入口模式下其余 5 块判成 none、失败块与不打分时的同一行完全一致、`entryScored.failedRequests` 与 health 都是 1；重建模式下失败块 429 重试用尽，其余 4 个 kept 块判成 none，`manifest.scorer` 是 `{ calls: 1, usage, fallback: false, failedRequests: 1 }`，reason 不含 `scorer:fallback`，health 的 `scorerFailedRequests` 为 1、`scorerFallbacks` 为 0；影子模式下只记 5 条判定、`failedRequests` 为 1；全部失败时重建仍追加 `;scorer:fallback:http:500`、入口保持全部 full，两处都不写 `failedRequests`。五条用例在旧实现上全部失败。重建部分失败的用例用 500，不碰熔断。
- 熔断两条（同一文件）：「stops starting requests on a rebuild once one exhausts its 429 retries, while in-flight requests finish and keep their decisions」——第一个请求持续 429，其余 3 个在途请求等它第 4 次 429 之后才应答；断言第 5 个块的请求一次都没发、在途 3 块判成 none、`failedRequests` 为 2、没有 fallback。「falls back with http:429 on a rebuild when every in-flight request exhausts its retries and the breaker stops the rest」——4 个在途请求各打 4 次共 16 次，第 5 个不发，`epochDecision.reason` 追加 `;scorer:fallback:http:429`。去掉熔断时两条都红。
- `packages/plugins/test/scorer/jev-calls.test.ts`：旧用例「一个块失败就丢掉全部判定」钉的是被淘汰的行为，改成断言成功块的判定与 usage 保留、`failedRequests` 为 1；新增「`signal` 中止后不再发新请求」，以及熔断的两条用例（见下）。
- `packages/core/test/health.test.ts`、`packages/core/test/log/registry.test.ts`、`apps/sage/test/bench.test.ts`：汇总、校验与 bench 指标。
- `pnpm check` 全绿。
