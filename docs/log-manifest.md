# Log entry 清单

由 `scripts/gen-log-manifest.mjs` 根据 `@sage/core` 的 `entryKinds()` 生成。手改会被 `pnpm check:log-manifest` 拒绝。

| kind | doc |
| --- | --- |
| leaf | Moves the leaf pointer to an existing entry. Does not copy history. |
| manifest | Compile manifest snapshot. The kernel does not interpret claims, chunks, or entryScored. |
| message | Conversation message. Payload is { role, content }. |
| model_change | Model switch recorded as an entry. History is not rewritten. |
| request_header | Request envelope, including the call's contextWindow and pricing (null when uncatalogued). system and tools are stored in full only when their hash differs from the previous request_header, or there is none. |
| subtask | Child session pointer. Names the parent leaf it recompiles. History is not copied. |
| terminal | Terminal outcome of a run. Optional usage and error stay opaque. |
| turn_retry | Whole-turn resend after a transport error or a transient in-stream error event. Not model-visible. Payload is { turn, attempt, reason }. |
| undo | Undo recorded as an entry pointing at an earlier id. History is not rewritten. |
| view_epoch | A new view epoch. The previous prefix is left intact. |
