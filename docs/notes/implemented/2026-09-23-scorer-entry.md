# 打分器入口生效

## Problem

影子打分（#65、#70）在 312 次调用里给出 full 93、summary 209、none 93，判丢的 83 个是 bash 输出。影子不改视图，模型永远不会 expand，这些判定准不准无法验证。打分器又只在阶段重建时真生效，而 DeepSeek 上重建几乎不回本，真生效的样本仍然是 0。

## Decision

`contextCompiler({ scorerApply: "entry", minScoreTokens })` 在工具块第一次进入视图时打分并定档。`minScoreTokens` 默认 300。候选是本轮新出现、当前仍是 full、且不是 `expand` 结果的工具块。`none` 留可 expand 的桩，`summary` 用不超过 200 字的笔记（没给则保持 full），`full` 原文不动。清单 reason 写成 `scorer:entry:<tier>`。抛错、`fallback` 或缺 id 保持 full，这次调用仍计数。判定写入当轮清单，之后由累积冻结保住。`keepRecent` 不再把已打分的块抬回 full，这些块也不再占最近窗口的名额。

`entry` 不跑重建路径上的打分，也不跑影子：同一次调用既写 `manifest.entryScored` 又改视图。默认仍是 `scorerApply: "rebuild"`，重建打分和影子保持原样。

`sage --scorer-apply entry` 必须同时给出 `--scorer model` 或 `--scorer jev`，否则退出 2。`bench run` 把 `--scorer` 和 `--scorer-apply` 透传给每个任务，`results.json` 顶层记下 `scorerApply`（没写旗标则是 `rebuild`）。`runHealth` 增加 `entryScored: { calls, none, summary, full, usage }`。

## Alternatives considered

为什么不是滑出最近窗口时再判。DeepSeek 缓存是前缀缓存。出窗口时改档，会让该位置之后的近期 6 轮每轮重新全价编码。按写入价 0.30 对缓存读价 0.006，要三百多轮才回本。入口定档则这块从第一次出现就是最终形态，前缀逐字节稳定。代价是模型偶尔需要全文时多一次 expand。

为什么不把影子档直接写回清单。影子要的是同一条轨迹上只观察、不干预。入口生效是另一条实验，默认仍是 rebuild。同一次编译里再跑一遍影子，会把同一次判定记两份。

## Consequences

确定性淘汰、阶段决策和校准没改。`expand` 的结果不进候选，否则刚取回的原文会在同一轮被桩掉。规则打分不改档，入口模式在 CLI 上拒绝它。没开 `entry` 的清单不带 `entryScored`，健康度里 calls 和 usage 是 0。子任务重编译不跑入口打分。

没加依赖。真机 A/B（`bench run --policy compiler --scorer model --scorer-apply entry` 对照 `20260923-0419-clean-passthrough`：解决率、tokensPerResolved、expandRate、cacheHitRatio）留给作者，这次不跑。

## Testing

`pnpm check` 通过：biome、tsc、分层、README、log manifest，vitest 76 个文件、373 个测试全绿。覆盖 8 轮入口判 none 后 `renderLlmInput` 为 `<last>` 加增量、不足 300 token 不调用且 reason 为 recent、抛错或缺 id 仍计 `entryScored.calls`、`--scorer-apply entry` 未给 model|jev 时退出 2。core 的两条 replay 记录补上了 `entryScored` 全 0，因为终端健康度现在总会带这个对象。测试不打真网，不跑真机 bench。
