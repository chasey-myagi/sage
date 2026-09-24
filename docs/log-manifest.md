# Log entry 清单

由 `scripts/gen-log-manifest.mjs` 根据 `@sage/core` 的 `entryKinds()` 生成。手改会被 `pnpm check:log-manifest` 拒绝。

| kind | persisted | reduce | doc |
| --- | --- | --- | --- |
| leaf | true | 无 | Moves the leaf pointer to an existing entry. Does not copy history. |
| manifest | true | 无 | Compile manifest snapshot. The kernel does not interpret claims, chunks, or entryScored. |
| message | true | 无 | Conversation message. Payload is { role, content }. |
| model_change | true | 无 | Model switch recorded as an entry. History is not rewritten. |
| request_header | true | 无 | Request envelope. system and tools are stored in full only when their hash differs from the previous request_header, or there is none. |
| subtask | true | 无 | Child session pointer. Names the parent leaf it recompiles. History is not copied. |
| terminal | true | 无 | Terminal outcome of a run. Optional usage and error stay opaque. |
| turn_retry | true | 无 | Whole-turn resend after a transport error. Not model-visible. Payload is { turn, attempt, reason }. |
| undo | true | 无 | Undo recorded as an entry pointing at an earlier id. History is not rewritten. |
| view_epoch | true | 无 | A new view epoch. The previous prefix is left intact. |
