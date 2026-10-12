/**
 * Text for a tool call left dangling by a crash.
 *
 * Guarantees: one model-facing sentence. The call started, its result was not
 * recorded, and the model must check external state instead of retrying
 * blindly. The Log cannot tell a call that never started from one that did,
 * so the kernel only ever claims the cautious case.
 *
 * Intentionally not in the Log: this function does not scan history and does
 * not append. Finding dangling calls is the view projection's job.
 */

import type { ToolResultBlock } from "@sage/provider/contract";

/** The model-facing text for a tool call whose result was never recorded. */
export const TOOL_OUTCOME_UNKNOWN =
  "This tool call was started, but its result was not recorded. Check the external state it may have changed before you decide what to do. Do not retry blindly.";

/** An error tool_result for `toolUseId` carrying `TOOL_OUTCOME_UNKNOWN`. */
export function synthesizeToolResult(toolUseId: string): ToolResultBlock {
  return {
    type: "tool_result",
    toolUseId,
    content: TOOL_OUTCOME_UNKNOWN,
    isError: true,
  };
}
