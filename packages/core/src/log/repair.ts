/**
 * Text for a tool call left dangling by a crash.
 *
 * Guarantees: two model-facing sentences. `TOOL_NOT_STARTED` means the call never
 * ran and a retry is safe. `TOOL_OUTCOME_UNKNOWN` means it did start, the
 * result was not recorded, and the model must check external state instead of
 * retrying blindly.
 *
 * Intentionally not in the Log: this function does not scan history and does
 * not append. Finding dangling calls is the view projection's job.
 */

import type { ToolResultBlock } from "@sage/provider/contract";

/** The two model-facing texts for a dangling tool call, keyed by cause. */
export const DANGLING = {
  TOOL_NOT_STARTED:
    "This tool call was not started before the session stopped. No effects were recorded. It is safe to retry.",
  TOOL_OUTCOME_UNKNOWN:
    "This tool call was started, but its result was not recorded. Check the external state it may have changed before you decide what to do. Do not retry blindly.",
} as const;

/** An error tool_result for `toolUseId` carrying `DANGLING[reason]`. */
export function synthesizeToolResult(
  toolUseId: string,
  reason: keyof typeof DANGLING,
): ToolResultBlock {
  return {
    type: "tool_result",
    toolUseId,
    content: DANGLING[reason],
    isError: true,
  };
}
