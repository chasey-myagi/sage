# Agent Note

## Problem

首个 tag 之前，包入口转出了一批全仓没有 import 方的符号，另有一批 `export` 只有本文件在用。其中两处带着文件头契约，但没有生产调用方：

- `core/src/testing/replay.ts` 的 `SAGE_RECORD=1` 录制档。它不录任何东西，只抛一句「没实现」；唯一读它的是一条断言它会抛错的用例。
- `plugins/src/routing/policy.ts` 的 `composePolicies`。文件头第一句契约讲的就是它，但 `apps/` 与 `packages/*/src` 里没有调用方，只有一条测试它自己的用例。

## Decision

- 删掉 `SAGE_RECORD` 分支、`RECORD_SWITCH` 常量、文件头与 `replaySession` JSDoc 里的对应句子，以及那条用例。`replaySession` 现在只有回放这一种行为。
- 删掉 `composePolicies`、文件头那句契约和它的用例。`withSensitiveFilter` 包一层 policy 的组合方式不受影响。
- 其余是不带契约的清理：入口转出、本文件专用的 `export`、`headHash`、`ContractVersion`、vitest 里冗余的 include。

## Alternatives considered

- 留着 `SAGE_RECORD` 当真机录制的占位。占位只会抛错，录制档真要做时，开关名、落盘格式都要重新设计，留一个只会抛错的分支帮不上忙。
- 留着 `composePolicies` 备用。G4 不让首个 tag 之前保留没人用的形状；真要串多个 policy 时，从 `RoutePolicy` 的类型就能写出来。

## Consequences

- 设 `SAGE_RECORD=1` 跑回放测试不再报错，照常回放。`docs/design/references.md` O1 里说的 `record` 档仍是设计目标，没有实现。
- 要串联多个 route policy 时得自己写，或者重新加回一个组合函数，并配上生产调用方。

## Testing

- `pnpm check` 全绿。
- `vitest list --filesOnly` 去掉 `regressions/**` 前后输出相同。
- 删掉的用例只钉被删代码本身：`refuses the record lane without calling the network`、`threads the previous model forward and keeps it when a later policy abstains`，以及 `head hashes` 用例里断言 `headHash` 的那一行。
