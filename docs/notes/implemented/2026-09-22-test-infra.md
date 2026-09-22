# 测试基础设施：无 key 回放、前缀感知输入快照、期望产物入库

## Problem

测试要回答「模型这一轮看见了什么」，但默认跑法仍可能带着 API key，也没有一份可以离线重放的会话，更没有把 system、tools、每轮请求和清单留在仓库里对照。断言如果只盯着某次实现的内部数组，换原语就全废。前缀一旦被破坏（例如把日期写进 system），也没有一条测试会当场变红。

## Decision

`@sage/core/testing` 是独立子路径，不进内核 index。`setup.ts` 只作为 vitest 的 setupFile：先删掉所有 `*_API_KEY`，再断言一个不剩，并把 `fetch` 换成抛错的替身。

回放的输入和 oracle 是同一份 `session.jsonl`，用 `JsonlLogStore` 读。`responses.json` 是按轮次的 `StreamChunk[][]`。`replaySession` 从 Log 里取回用户原文、模型和 `adapterDefaults`，用 faux adapter 重跑。比较 Log 时丢掉 `id` / `parentId` / `at` 和 `details.responseId`。`SAGE_RECORD=1` 只表示录制档，直接抛错；真机网络录制留在 #16。

`renderLlmInput` 把一次请求印成 `system` / `tools` / `messages`。head 相同且上一次的 messages 是本次的 JSON 前缀时，输出是 `<same>`、`<last>` 和增量；否则整段重印。日期进 system 的那一轮用内联快照钉住。

`system-prompt.md`、`tool-schemas.json`、`turn-<n>.request.json`、`manifest-turn-<n>.json` 提交在 case 的 `expected/` 里。文件不存在或 `UPDATE_EXPECTED=1`（`pnpm test:update`）才重写，否则字节不同就抛出带 diff 的错误。manifest 里的 chunk id 写成 `seq:<n>`，因为 Log 的 id 每次回放都变。

覆盖率 `include` 只圈 `packages/core/src/view/**` 和 `packages/core/src/session.ts`，provider 配 `v8`，阈值不设。回归测试放在 `test/regressions/issue-<n>.test.ts`，`issue-0.test.ts` 只占名字。

## Alternatives considered

期望产物如果只放内联快照，短的反例（日期写进 system）很合适，但 system prompt、tool schema、整轮 request 和 manifest 一长，diff 就混在测试源码里，审 PR 的人看不到产物本身。单独入库之后，实现变了，红的是那份文件的逐行 diff，`test:update` 再重写。

只 mock 适配器、不剥凭据，挡得住这一次假的 `stream`，挡不住测试或依赖去读 `process.env` 里的 key，也挡不住有人直接 `fetch`。启动时把 key 从进程里删掉，再让 `fetch` 一调用就失败，漏掉的真网请求会红，而不是悄悄打出去。

## Consequences

回放不打开网络。`SAGE_RECORD=1` 不会录，只会说明格式并指向 #16。`contextWindow` 固定 8000、`pricing` 为 null，Log 里没有这两项。

manifest 期望文件不保存 uuid。比的是轮次、档位、原因和异常。JSON 期望文件按 biome 的格式写入，这样 `biome check` 折行之后仍然和测试打印的字节一致。

`@vitest/coverage-v8` 不在依赖里。`pnpm test --coverage` 因此失败。本票不加这个包，也不把 provider 改成 istanbul（那个包同样没装）。阈值等 M2 编译器落地再设。

## Testing

`pnpm check` 退出码 0：lint、typecheck、分层、README、log manifest，以及 22 个文件 90 个测试。红灯时 testing 入口不存在（`Cannot find module '../../src/testing/index.ts'`），设了 `*_API_KEY` 的进程在 setup 接上之前断言失败。把 `system-prompt.md` 改成别的内容后，测试打印 `--- expected` / `+++ actual` 的 diff；`UPDATE_EXPECTED=1` 把文件写回。`pnpm test --coverage` 的原文是 `MISSING DEPENDENCY  Cannot find dependency '@vitest/coverage-v8'`。
