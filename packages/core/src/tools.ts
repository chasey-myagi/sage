/**
 * Tool interface the session knows how to call.
 *
 * Guarantees: the kernel does not ship tools. A tool's schema is what the
 * view puts on the head, and `execute` is its only side effect. Only the
 * tool_result content and error flag are model-visible.
 *
 * Intentionally not in the Log: Access lists and the tool object.
 */

import type { ToolDef, Usage } from "@sage/provider/contract";

/** One resource a call would touch, for sandbox and conflict checks. */
export type Access = {
  key: string;
  mode: "read" | "write" | "search";
  recursive?: boolean;
};

/**
 * One nested session a tool call ran. A tool that runs child sessions lists
 * them as `details.childRuns: ChildRun[]`; `runHealth` counts each item and
 * sums its `usage` under `subtasks`, and nothing else reads them. Extra
 * fields are the tool's own and stay opaque to the kernel.
 */
export type ChildRun = { usage: Usage } & Record<string, unknown>;

/**
 * A tool as the session sees it. The three `is*` flags describe its risk.
 * `accesses` reports what a given input would touch without running it.
 */
export interface HarnessTool {
  def: ToolDef;
  isReadOnly: boolean;
  isDestructive: boolean;
  isOpenWorld: boolean;
  accesses(input: unknown): Access[];
  /**
   * Runs the call. `ctx.toolUseId` is the tool_use id. A failure should be
   * returned with `isError: true`; a throw is also turned into an error
   * result. `details` is not model-visible: the kernel stores it on the user
   * message entry as `details.toolResults`, not on the tool_result block.
   */
  execute(
    input: unknown,
    ctx: { sessionId: string; toolUseId: string; signal?: AbortSignal },
  ): Promise<{
    content: string;
    isError: boolean;
    details?: Record<string, unknown>;
  }>;
}
