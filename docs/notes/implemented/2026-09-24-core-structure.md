# 内核与 provider 的结构与注释整理

## Problem

`session.ts` 的 `turn()` 116 行，编译视图、切模型、写请求头、流式调用、跑工具五件事写在一个函数里；`runTools` 在 for 里用三元套对象字面量再套 async 箭头，嵌套六层。文件头 65 行，是按 PR 追加的逐步流程叙述，读者找不到某句话对应哪段代码。`drive.ts` 的 `driveStream` 123 行，「发出 aborted 结束块」同一段代码写了五遍。`hooks/dispatcher.ts` 的取消订阅闭包写了五遍，observer 靠 `payload as never` 调用。两个 adapter 的构造函数与 `stream()` 几乎逐行相同，openai 的 `openThinking` / `openText`、`applyReasoning` / `applyContent` 两两互为复制。`log/kinds.ts` 文件头 53 行，关于 `request_header` 何时存全文的说法（「与默认值不同时」）与 `session.ts` 的实际行为（与上一条 request_header 的 hash 不同，或没有上一条）不一致。`catalog.ts` 有中文代码注释，被所有已知模型共用的 `8192` 没有说明是什么。`head` 一词三义。测试替身有 `test/support/faux.ts` 与 `src/testing/faux.ts` 两套。注释里留着 `(#4)`、`(#5)`、`(#16)` 这类裸编号，「no high-water mark」在五个文件头各说一遍。

## Decision

**session.ts 拆分。**`turn()` 只剩编排（41 行），每一步是一个有名字的私有方法，逐步流程写在各自的 JSDoc 上：`compileView`（取视图、配对与冻结断言、`view_compiled` 深拷贝）、`applyRoute`、`recordRequest`（view_epoch / request_header / manifest 三条条目，返回请求）、`recordAssistant`、`settleAssistant`（按 finish 决定结局并跑工具）、`endTurn`。原文件头 23 条事实逐条落到了对应方法或类型上（`RunSummary` 字段、构造函数、`run`、`routeReason`、`appendViewEpoch`、`appendHeader` 等），文件头只留四条对调用方的保证和「故意不进 Log」。文件从 701 行降到 578 行，靠拆出三个新模块：

- `core/src/turn-retry.ts`：`collectWithTurnRetry`、`TURN_RETRY_LIMIT` / `TURN_RETRY_BASE_MS`、传输错误判定。
- `core/src/tool-run.ts`：`runToolCalls`、`toolUses`、`ToolResultDetails`；工具 `details` 的收集改成顶层函数 `recordingDetails`，嵌套从六层降到两层。
- `core/src/log/path.ts`（分支 1 的 `log/epoch.ts` 改名扩充）：`viewEpochs`、`latestViewEpoch`、`lastHeaderIndex`、`lastRequestHeader`。

`structuredClone(...) as StoredEntry[]` 与 `sampling as Record<string, unknown>` 两处断言删掉，类型本来就对。`markedTransport` 已在上一提交删除，没有残留。

**请求头 hash 比对。**两处条件语义不同，不是同一个判断做了两次。`assertHeadFrozen(view, prev, frozen)` 要求 `view.epoch > frozen`：视图的阶段比上一次请求所在的阶段新。session 另加的 `view.epoch !== latest` 要求视图的阶段就是路径上已经记下的最新 view_epoch：视图不能声称一个 Log 里还没有的阶段。只有第二条时，视图报一个 Log 里没有的阶段号就能换 head；只有第一条时，在上一次请求所在的阶段里也能换 head。合起来，head 只能在 `view.epoch === latest > frozen` 时改变。两处的 hash 比对本身合并成 `view/assert.ts` 私有的 `headMatches`；session 的那一半挪进新导出的 `assertHeadFrozenOnPath(view, path)`，它先做第一部分再调 `assertHeadFrozen`。JSDoc 写清了两部分的区别，以及由此推出的一个后果：session 在断言之后才追加 `epochStart` 的 view_epoch，所以借 `epochStart` 开阶段的那一轮不能同时换 head。行为不变，`assertHeadFrozen` 签名不变。

**drive.ts。**`driveStream` 拆成：开请求（`openWithRetry`）、读 body 的 `readBody`（产出内容块、**返回**结束块）、`openFailure`、`abortedFinish`。结束块只在一处发出，`sealed` 标志随之删除。123 行降到 31 行。

**dispatcher。**五个取消订阅闭包合成 `register(list, item)`。observer 表改成按事件预建的映射类型 `{ [E in keyof HookEvents]: ObserverFn<E>[] }`：TypeScript 能经泛型键读映射类型，不能经它写，所以九个事件的空列表在字段初始化时建好，`payload as never` 与 `fn as Observer` 都不需要了。`on` 的实现签名改为元组联合 `["tool_pre", PreFn] | ["tool_post", PostFn]`，解构后按判别字段收窄，`fn as PreFn` / `fn as PostFn` 也删了。

**adapter 去重。**`http.ts` 新增 `Client` 与 `resolveClient(options, defaultBaseUrl)`，两个 adapter 各只存一个 `client` 字段；`driveStream(client, call)` 接收协议无关的部分，`SseCall` 只含各协议不同的四样：路径、请求头、请求体、事件解码器。JSON 事件解析合成 `record.ts` 的 `parseRecord`。openai 的 `openThinking` / `openText` 合成 `openBlock(state, driver, kind, id?, name?)`（`applyTool` 也用它分配块号），`applyReasoning` / `applyContent` 合成 `applyText(state, driver, kind, text)`。保留下来的差异都是协议决定的，写在两个类的 JSDoc 上：路径、鉴权头（`x-api-key` 加 `anthropic-version` 对 bearer）、请求编码、解码器（OpenAI 线上没有块号，解码器自己分配）。`anthropic-version` 提成具名常量 `ANTHROPIC_VERSION`。

**常量。**`catalog.ts` 的 `KNOWN_MAX_TOKENS = 8192` 是报告为 `adapterDefaults.maxTokens` 的单次输出上限（线上的 `max_tokens`），与上下文窗口无关，所以百万上下文的模型也共用它；`FALLBACK_WINDOW`、`FALLBACK_MAX_TOKENS` 同样补了 JSDoc，中文注释改成英文。`testing/replay.ts` 与两套替身里的 `8000` 合成 `FAUX_CONTEXT_WINDOW`。`"stream ended before completion"` 在 collect、http、drive 出现四次，提成 contract 的 `INCOMPLETE_STREAM`。

**类型与术语。**`ChunkRecord` 与 `ClaimRecord` 字段相同、语义相同（manifest 上一行档位决定：id、档位、理由、token 数），改为一个 `TierRecord` 加两个别名，档位联合提成 `Tier`，都从 `@sage/core` 导出。`view/types.ts` 在对应类型上定义了 chunk、tier（四档各是什么形态）、claim、epoch、stub、manifest、payback / alpha / expectedTurns（`EpochDecisionReport`）、影子打分与入口打分。`BudgetReport` 补上编译器一直在写、类型里却没有的 `omitted`（本轮 none 档块数）与 `expands`（路径上 expand 调用的累计数），可选字段，运行时不变。

**head。**只改了纯代码内的标识符：`appendViewEpoch` 里指 `messages[0]` 的局部变量改名 `firstMessage`。写进 Log 的 `details.epochHead`、`calibration.head` 不改名，在 `Head`、`ViewEpochPayload`、`CalibrationReport.head` 的 JSDoc 写清含义；持久化字段怎么改见 [head 命名](./2026-09-24-head-naming.md)。

**注释。**core 与 provider 每个 `src` 文件的文件头改成职责、保证、`Intentionally not in the Log:` 三段，都不超过 25 行（`session.ts` 65→22，`log/kinds.ts` 53→14，`health.ts` 30→12，`view/types.ts` 28→16）；`kinds.ts` 的逐字段校验规则挪到各 payload 类型上，`health.ts` 的计算口径挪到 `RunHealth` 的字段上。每个导出符号补了 JSDoc。裸 issue 编号换成结论。「no high-water mark」只留在 `log/jsonl.ts`，并解释了是什么。`log/kinds.ts` 的文件头、`RequestHeaderPayload` 与 `request_header` 的 `doc` 改成与 session 实际行为一致，`docs/log-manifest.md` 随之重新生成。

**测试替身。**两套合并到 `src/testing/faux.ts`：`fauxAdapter` 接受函数脚本（原先只有 `test/support` 那套支持），`createFaux`、`stopScript`、`toolScript` 与 `FAUX_MODEL` / `FAUX_SYSTEM` / `FAUX_USAGE` 从 `@sage/core/testing` 导出。`createFaux` 保留原语义：`prepare` 总是返回同一个 `prepared` 对象。core 与 tools 的测试只改了 import。

## Alternatives considered

把 session 的 `view.epoch !== latest` 并进 `assertHeadFrozen`：要改一个已导出、有直接测试的函数的签名，而且会把两条不同的条件藏进一个参数表里。保留 `assertHeadFrozen` 原样、在外面包一层读 path 的函数，两条条件各有名字。

adapter 用抽象基类共享构造与 `stream()`：规范说 class 只给持有可变状态的长生命周期对象，而 adapter 共享的只是一份配置和一次函数调用，组合一个 `Client` 值更直白。

`test/support/faux.ts` 直接删除：apps/sage 与 plugins 编译器的测试还从这个路径导入，这两块正由别人重构，本分支不碰。所以它暂时变成一个只做转出的文件，文件里写明删除条件。

`driveStream` 保持原结构、只合并 abort 分支：仍超过 60 行，而且 `sealed` 标志与五个出口的配合正是难读的根源。

## Consequences

`driveStream` 的结束块现在在 try 之外发出。唯一的可观察差异是调用方对生成器调用 `.throw()` 且恰好停在最后两个块上时：原来被 `sealed` 吞掉，现在抛给调用方。仓内没有任何调用方用 `.throw()`。读 body 时 `return` 发生在结束块发出之前，所以 reader 的 cancel 比原来早一步，调用方拿到的块序列不变。

`@sage/core` 新增导出 `Tier`、`TierRecord`；`@sage/core/testing` 新增 `createFaux` 等。`test/support/faux.ts` 与 apps / plugins 测试的 import 迁移留给后续小 PR，届时删掉这个文件。

`testing/replay.ts` 的 `SAGE_RECORD=1` 报错文案仍含 `#16`：`replay.test.ts` 断言 `/#16/`，本分支不改断言。

## Testing

`pnpm check` 退出码 0：lint、tsc、分层自检、README、log manifest、87 个测试文件、418 个测试，与分支 1 相同。`git diff` 显示测试文件只改了 import，外加 `test/support/faux.ts` 变成转出文件；没有改断言，没有删用例。`docs/log-manifest.md` 只有 `request_header` 一行的 `doc` 文本变化。
