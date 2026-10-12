# Log 内核瘦身

## Problem

Log 内核留着几处没人用的扩展点，读起来像有人在用：

- `log/store.ts` 的 `replay` 按 seq 调用注册表里的 `reduce`。十个内核 kind 没有一个带 reducer（`kinds.ts` 文件头自己写着），`replay(` 唯一的调用方是 `memory.test.ts` 里注册测试专用 kind `test.tally` 的那条用例。`EntryKindDef` 为它多带一个泛型 `S`。
- `EntryKindDef.persisted` 的类型是字面量 `true`，只能填 true；唯一的读者是 `scripts/gen-log-manifest.mjs`，在清单里印出一列全是 `true` 的 `persisted` 和一列全是「无」的 `reduce`。
- `log/repair.ts` 有两句模型可见文本。生产唯一的调用是 `view/passthrough.ts` 的 `synthesizeToolResult(id, "TOOL_OUTCOME_UNKNOWN")`；`TOOL_NOT_STARTED` 只有 `repair.test.ts` 在用。录制夹具（`packages/*/test/**` 下全部 jsonl 与 `responses.json`）里两句文本都没有出现过。Log 不记「工具已开始执行」，投影层本来就分不出「没开始」。
- `AgentSession` 构造同时接受 `adapter` 与 `adapters`，用交叉联合类型表达，构造里再把单个 `adapter` 合并进表。生产三处（`apps/sage/src/coding-session.ts` 两处、`plugins/src/subtask/spawn.ts`）都传 `adapters`；单 `adapter` 只有 `testing/replay.ts` 和测试在用，读起来像兼容层。

## Decision

- 删掉 `replay`、`EntryKindDef.reduce` 与泛型 `S`，`registerEntryKind<P>` 只剩一个类型参数。删掉 `memory.test.ts` 里测 `replay` 的那条用例。
- 删掉 `persisted` 字段和十个 kind 上的 `persisted: true`。清单生成器去掉 `persisted` 与 `reduce` 两列，`docs/log-manifest.md` 重新生成，只剩 `kind | doc`；各 kind 的 doc 文本不变。
- `DANGLING` 对象换成单个常量 `TOOL_OUTCOME_UNKNOWN`，`synthesizeToolResult(toolUseId)` 去掉 `reason` 参数。文本字节不变：与 `HEAD` 的字面量逐字节比较一致，`repair.test.ts` 新加一条 `toBe` 把全文钉住。
- `AgentSession` 只收 `adapters`。`testing/replay.ts` 与 23 个测试文件的 50 处构造机械替换为 `adapters: { <provider>: adapter }`，key 取该处 model 的 provider（`FAUX_MODEL` 与各文件的 anthropic `MODEL` 用 `anthropic`，`transport-flag.test.ts` 的 `MODEL` 用 `openai`；model 是变量的三处用 `[model.provider]`）。`model-change.test.ts` 四处本来就有含 anthropic 的 `adapters`，只删掉多余的 `adapter`。
- app 层同样的两种形状一并收掉：`createCodingSession` 的选项与 `CliIo` 只留 `adapters`。生产唯一的 `createCodingSession` 调用（`print.ts`）本来就传 `adapters`；`CliIo.adapter` 是测试 seam，生产从 env 组装。`apps/sage/test` 7 个文件 23 处改成 `adapters: { <provider>: adapter }`，key 取该次 `runCli` 解析出的 provider（只设某家 key 时就是那家；`ANTHROPIC_API_KEY` 与 `OPENAI_API_KEY` 同时设时按现有断言是 `openai`）或 `createCodingSession` 的 `model.provider`。注入的表不服务选中的 provider 时，`assembleAdapters` 直接抛错，不再补一个会打真网的 adapter；只剩 `adapters` 一种写法后 key 写错是唯一的失误方式，测试 seam 必须 fail-closed（用例 `rejects injected adapters that do not serve the selected provider instead of reaching the network`，修之前它在 1 秒超时上红）。`requireAdapters` 删掉，换成一处 provider 检查。
- `docs/design/references.md` 的 L2、V3 按现状改写并链到本 Note；`core/README.md` 删掉「也可以继续传单个 `adapter`」。

**Log entry 的形状不变。**`persisted` 与 `reduce` 只活在进程内的注册表里，从来不写进 entry；`EntryInput` / `StoredEntry`、各 kind 的校验器、JSONL 行格式都没动。**模型可见文本不变**：passthrough 补的悬空结果仍是同一句 `TOOL_OUTCOME_UNKNOWN`。

## Alternatives considered

- 保留 `DANGLING` 对象只剩一个键：调用方仍写 `DANGLING.TOOL_OUTCOME_UNKNOWN`，改动更小，但一个键的查找表暗示还有别的原因可选。不选。
- 保留单 `adapter` 作为测试便利：测试便利不该写进生产构造的类型，`@sage/core/testing` 若需要可以提供替身工厂；目前一行 `adapters: { anthropic: adapter }` 已经够短。
- 只删清单的两列、保留类型字段：字段没人读之后就只剩误导。

## Consequences

以后要加回这些扩展点需要的东西：

- **reducer**：某个 kind 真需要内核折叠时，在 `EntryKindDef` 上加 `reduce` 与状态泛型，加回 `replay`（或在使用方就地折叠，plugins 现在都是这么做的），清单加一列。要写明折叠结果是不是同一事实的第二个来源（L5）。
- **非持久 kind**：要表达「不落 Log 的 entry」时，按 L5 它本来就该是运行时事件而不是 entry，不应加回 `persisted: false`。
- **`TOOL_NOT_STARTED`**：前提是 Log 能区分「工具没开始」与「开始了但没记结果」，例如执行前追加一条开始标记。那是新的 entry kind，要随 Note、清单和夹具一起加；在那之前补这句文本等于对模型说一句 Log 证明不了的话。
- 新的测试构造 `AgentSession` 必须写 `adapters`，key 与 model 的 provider 不一致会在构造时抛 `No adapter for provider`。

## Testing

- `pnpm check`：biome、`tsc -b`、CLI 打包、分层、export JSDoc、README、log manifest 新鲜度、vitest 94 个文件 500 个用例、打包冒烟，全绿。
- 删掉的用例：`memory.test.ts` 的 `folds reducers in seq order and skips kinds that have none`（测被删的 `replay`）；`repair.test.ts` 里关于 `TOOL_NOT_STARTED` 的 `toEqual` 与两条 `toMatch`（测被删的文本），同一条用例里 `TOOL_OUTCOME_UNKNOWN` 的断言原样保留并加了全文 `toBe`。
- Log 形状：core 的 `text-only` / `tool-roundtrip` 录制回放（`session.jsonl` 期望产物逐字比较）与 plugins `issue-46` / `issue-50` / `issue-63` 夹具回放不改夹具、不重新生成期望产物，全部通过；`JsonlLogStore` 重开时按注册表校验每一行，同样通过。
- adapter 改写（core、plugins、tools 与 apps 两批）：对 diff 过滤 `expect` 行为空，构造写法以外没有断言变化。
