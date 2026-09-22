/**
 * Tool surface the session knows how to call.
 *
 * Contract: the kernel does not ship tools. A tool's schema is what the view
 * puts on the head. execute is the only side effect. ctx.toolUseId is the
 * tool_use id for this call. Only the tool_result content is model-visible.
 *
 * Intentionally not in the Log: Access lists and the tool object. execute's
 * `details` are not model-visible. The kernel stores them on the user
 * message entry as `details.toolResults`, not on the tool_result block.
 */

import type { ToolDef } from "@sage/provider/contract";

export type Access = {
  key: string;
  mode: "read" | "write" | "search";
  recursive?: boolean;
};

export interface HarnessTool {
  def: ToolDef;
  isReadOnly: boolean;
  isDestructive: boolean;
  isOpenWorld: boolean;
  accesses(input: unknown): Access[];
  execute(
    input: unknown,
    ctx: { sessionId: string; toolUseId: string; signal?: AbortSignal },
  ): Promise<{
    content: string;
    isError: boolean;
    details?: Record<string, unknown>;
  }>;
}
