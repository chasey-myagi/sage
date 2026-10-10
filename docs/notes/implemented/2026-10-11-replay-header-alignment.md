# 离线回放从 request_header 取窗口与定价，与会话共用落盘

## Problem

`replayOffline` 写死 `REPLAY_WINDOW = 8000` 和 `pricing: null`，线上会话则从 catalog 取窗口与定价（deepseek-flash 是 1M）。编译器的分叉守卫看窗口，成本不等式看定价，所以同一份 Log 离线回放出的 rebuild 决策和真机不可比：issue-63 那条 36 轮 deepseek 轨迹在回放里第 6 轮撞上 `context-window-guard` 并重建，真机没有；每一轮的 reason 都多出 `;pricing:fallback`，payback 轮数也全错（真机 `payback:106>13`，回放 `payback:25>13`）。根因是 `request_header` 不记这两项，catalog 手工改一次价，旧会话的回放路由也跟着变。

另外 `offline.ts` 的 `commitView` 自己拼 `view_epoch` / `request_header` / `manifest`，`request_header` 每一轮都带 system 与 tools 全文，没有按 L4 只在首见或 hash 变化时存。`issue-50` 与 `issue-63` 两个回归测试里还各抄了一份同样的 `commitView`。

## Decision

`RequestHeaderPayload` 新增必填的 `contextWindow`（正的有限数，token）和 `pricing`（`Pricing | null`），值是这次 `PreparedCall` 同名字段的快照。校验器拒绝缺字段、窗口不为正、定价形状不对的 header；G4，旧形状直接校验失败。`Pricing` 的守卫 `isPricing` 放在 `@sage/provider/contract`，和类型同处。

view_epoch、request_header、manifest 三条的落盘抽成 core 导出的 `appendRequestEntries(store, sessionId, { view, model, sampling, prepared })`（`packages/core/src/view/commit.ts`）。内容寻址规则只在这里实现一次。`AgentSession.recordRequest`、`replayOffline` 的影子 Log、两个回归测试都调它；会话里原来的 `appendViewEpoch` / `appendHeader` 删掉。

`replayOffline` 每一轮从该轮录下的 `request_header` 读窗口与定价，`REPLAY_WINDOW` 删掉。影子 header 的 `sampling` 也取录下的值，不再写 `{}`。`Policy` 新增 `alpha`：自适应视界之前录下的会话要用 `alpha: 0` 才能复现当时的 `remainingTurns`，否则第 15 轮以后的 `payback:N>M` 的 M 必然不同。`replayOffline` 的 deps 新增测试缝 `shadow`，生产环境用新建的 `MemoryLogStore`。无 key 回放 `replaySession` 同样改为从录下的第一条 header 读窗口与定价，不再写死 `FAUX_CONTEXT_WINDOW`。

fixture 迁移用一次性脚本（不入库），每条 request_header 在 `adapterDefaults` 之后插入两个字段，其余字节不动：

- `packages/plugins/test/replay/fixtures/{issue-46,issue-50,issue-63}`：录制模型是 deepseek-flash，填 `catalog.ts` 的真实值：窗口 1,000,000，定价 input 0.3 / output 1.2 / cacheWrite 0 / cacheRead 0.006，source 为 DeepSeek 定价页。依据：三份录制时间是 2026-09-23 02:09–03:48（+08:00），这段时间 catalog 的 deepseek-flash 条目（#42 加入、#45 改过写价口径之后）与现在逐字段相同；录下的 manifest 里没有一条 reason 带 `;pricing:fallback`，也没有 `context-window-guard`，说明录制时走的就是目录定价和大窗口。
- `packages/core/test/replay/{text-only,tool-roundtrip}`：模型 `claude-test` 不在 catalog 里，catalog 回退值是 128000 与 null。这两份不是真机录的，是 `fauxAdapter` 录的，当时的 prepared call 是 `FAUX_CONTEXT_WINDOW`（8000）和 null，所以填 8000 与 null，不用 catalog 回退值。

## Alternatives considered

只在回放侧按 provider/model 查当前 catalog：线上线下能对齐一时，catalog 一改价，旧会话的回放决策照样跟着变。issue 要的是「Log 里有当时的事实」，L4 也要求模型请求的输入能从 Log 重建。

把落盘函数放在 plugins：session 在 core，core 不能 import plugins。放在 `log/` 下也不合适，它要 `CompiledView` 和 `systemHash`，是 view 层的东西；`view/commit.ts` 是 session 和 plugins 都够得到的最低一层。

回归测试用现场跑一遍 `AgentSession` + faux 适配器产生的 Log：能对齐，但不是真机录下的轨迹。issue-63 的轨迹加 `alpha: 0` 就能逐轮复现，用真机数据更有说服力。

issue-46 最后一个用例改为断言 1M 窗口下的新结果：它钉的是 8000 窗口下的安全阀重建（`context-window-guard` 只在第 4 轮），见 `2026-09-23-cumulative-freeze.md` 的「回放窗口仍是 8000、定价 null」。换成大窗口就测不到安全阀了，所以保留断言，让用例显式构造小窗口的 header。

## Consequences

每条 `request_header` 多两个字段，定价对象每轮都存一份全文（几十字节，不做内容寻址）。旧会话 Log 打不开，预发布不迁移。离线回放的阶段决策只取决于 Log 和回放策略，不再取决于当前 catalog。

依赖 8000 窗口的测试改为显式构造：

- `issue-46` 最后一个用例把 fixture 拷进内存 store，覆写每条 header 为 `contextWindow: FAUX_CONTEXT_WINDOW`、`pricing: null`（原来回放写死的那组值），断言不变；用例名加上「under an 8000-token window」。
- `issue-50` 本来就在自己的 `viewContext` 里写了 8000，只把它抄的 `commitView` 换成 `appendRequestEntries`。`issue-63` 本来就写了 1M 与 DeepSeek 定价，同样只换落盘。
- `offline.test.ts` 与插件、core 的几个测试手写的 header 补上 `contextWindow: FAUX_CONTEXT_WINDOW`、`pricing: null`，和原来回放写死的值一致。

`rebuild.ts` 的 `rebuildContext` 仍用 `contextWindow: 0`，那里只做直通投影、不写 Log，不在本次范围内。

## Testing

`packages/plugins/test/regressions/issue-133.test.ts` 先红后绿：

- 「replays a recorded deepseek session to the same epoch decision on every turn」：issue-63 轨迹按 budget 8000、keepRecent 6、expectedTurns 20、`alpha: 0` 回放，36 轮的 `{turn, rebuild, reason}` 与录下的 manifest 逐条相等。修复前第 1 轮起 reason 带 `;pricing:fallback`，第 6 轮 `context-window-guard` 且 `rebuild: true`，第 8 轮 `payback:25>13`（录下的是 `payback:106>13`）。
- 「writes system and tools only on the first replayed request_header when the head is unchanged」：tool-roundtrip 两轮回放，第二条影子 header 没有 `system` 与 `tools`，hash 与第一条相同。修复前第二条带 `"You are sage. Use tools when asked."`。
- 「copies the recorded context window and pricing onto the replayed request_header」：36 条影子 header 的窗口与定价和录下的逐条相等，第一条等于 catalog 里 deepseek-flash 的值。修复前是 `contextWindow: undefined`。

另加：`registry.test.ts` 覆盖缺窗口、窗口为 0、缺定价、`source` 不是字符串、`currency` 不认识时校验失败；`session/request.test.ts` 覆盖会话把 prepared 的窗口与定价写上 header。`pnpm gen:log-manifest` 更新了 `docs/log-manifest.md` 的 request_header 一行。`pnpm check` 退出码 0。
