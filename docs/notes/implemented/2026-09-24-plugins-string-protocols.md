# plugins 剩余的字符串协议换成结构化字段

## Problem

编译器拆分后，plugins 里还有几处逻辑判断读字符串：

- `scorer.name === "rule"` 识别规则打分器，出现在重建打分、入场打分与离线重放三处。打分器的 `name` 是写进 Log 的标签，换个名字判断就悄悄失效。
- 重建打分报告的 `calls` 是 0 还是 1，靠 `fallback === "no-previous-request" || fallback === "no-adapter"` 判断。fallback 是给人和 Log 看的原因文字，新增一种「发请求前就放弃」的原因时，这里必须同步记得改。
- `replayOffline` 在返回数组上挂一个不可枚举的 `scorerNote` 字符串，`compareOffline` 再比较它是否等于 `scorer:offline-rule` 来判断打分器是否被换掉。
- 工具名 `"expand"` 在三个文件里各写一遍，`"update_ledger"` 在同一文件写两遍，读 Log 时按字面量匹配。

## Decision

纯重构，写进 Log 与报告的字符串逐字节不变。

- `Scorer` 加可选判别属性 `isNoop?: true`，只有 `ruleScorer` 设置。含义：不发任何调用、每个候选都判 full。重建打分据此跳过改写，入场打分据此不调用，离线重放据此保留而不替换。
- `ScoreResult` 加 `requested?: false`。模型打分器在 `no-previous-request` 与 `no-adapter` 两条发请求前的退路上设置它；重建打分报告读它决定 `calls`。`score-rebuild.ts` 里与 `ScoreResult` 重复的 `ScoreReply` 类型删掉。
- `offline.ts` 用模块级 `WeakSet` 记住「打分器被替换过」的返回数组，`compareOffline` 查集合后写入同一条 note。去掉了 `Object.defineProperty` 与读回时的类型断言。
- 新增 `EXPAND_TOOL_NAME`（`compiler/expand.ts`）与 `LEDGER_TOOL_NAME`（`claims/progress-ledger.ts`），工具定义与读 Log 的地方共用。

检查范围：`packages/plugins/src/**` 与 `packages/tools/src/**` 里所有 `.includes(` / `.startsWith(` / `.endsWith(` / `.match(` / `.test(` / `indexOf(` / `=== "` / `!== "`。其余命中都不是字符串协议：

- 比较字面量联合字段（`kind`、`type`、`role`、`tier`、`placement`、`mode`、`network`、`platform`、`trigger`、`source`、`outcome` 等），类型系统已经约束取值。
- 解析外部输入：preset 文件的 frontmatter、模型回复里的 JSON、路径 `..` 判断、二进制文件的 `\0` 探测、errno `ENOENT`。
- `reasons.ts` 的 `reasonKind`：从 Log 读回的块决策只剩 reason 字符串，它是编译器拆分时定下的唯一解码点。
- 从块 id 里按 `#` 取 `toolUseId`（`rebuild.ts`、`offline.ts`）：块 id 的格式本身是 Log 里的标识，不是状态。
- `score-shadow.ts` 的 `text !== "undefined"` 属于 `errorText` 副本，由统一下沉的 PR 处理。

## Alternatives considered

把 `isNoop` 做成必填，或把 `Scorer` 做成 `kind` 判别联合：测试里有十几个内联打分器替身，要逐个补字段。可选的字面量 `true` 已经够用：缺省即「会调用、会改写」，只有规则打分器需要声明例外。

把 `ScoreResult` 做成 `ok` / `fallback` 判别联合：同样要改所有替身与 `jev.test.ts` 里对整个结果的 `toEqual` 断言，而重构不许改断言。

按 fallback 的前缀或集合常量判断 `calls`：还是字符串协议，只是换了写法。

## Consequences

新写一个打分器，只要在「发请求前就放弃」的路径上返回 `requested: false`，报告的 `calls` 自动为 0，不用改编译器。

两处现有的不一致保持原样，行为修改另开 PR：

- `jevScorer` 缺 API key 时返回 `no-api-key`，也没发请求，但没有设置 `requested`，重建打分报告仍记 `calls: 1`。`jev.test.ts` 对这个结果做整对象比较，改它就要改断言。
- 入场打分遇到任何 fallback 都记一次调用，包括模型打分器的 `no-adapter`；重建打分同一情况记 0。

## Testing

`pnpm check` 退出码 0（lint、tsc、分层、README、log manifest、87 个文件、419 个测试）。测试文件一行没改。
