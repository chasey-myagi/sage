/**
 * Runs one assistant message's tool calls through the hook pipeline.
 *
 * Guarantees: calls run one at a time, in message order, and a tool
 * failure becomes an error tool_result instead of a throw. A tool's
 * `details` are collected apart from its tool_result block, which is all
 * the model sees.
 *
 * Intentionally not in the Log: nothing here appends. The session stores
 * the results as one user message and the details as its
 * `details.toolResults`.
 */

import type {
  ContentBlock,
  ToolResultBlock,
  ToolUseBlock,
} from "@sage/provider/contract";
import type { HookDispatcher } from "./hooks/dispatcher.js";
import type { HarnessTool } from "./tools.js";

/** One tool's `details`, stored on the user message as `details.toolResults`. */
export type ToolResultDetails = {
  toolUseId: string;
  name: string;
  details: Record<string, unknown>;
};

/** The tool_use blocks of `content`, in order. */
export function toolUses(content: ContentBlock[]): ToolUseBlock[] {
  return content.filter(
    (block): block is ToolUseBlock => block.type === "tool_use",
  );
}

/**
 * A copy of `tool` whose execute also pushes a non-empty `details` onto
 * `sink`. Those details never reach the model-visible tool_result block.
 */
function recordingDetails(
  tool: HarnessTool,
  call: ToolUseBlock,
  sink: ToolResultDetails[],
): HarnessTool {
  return {
    ...tool,
    execute: async (input, ctx) => {
      const raw = await tool.execute(input, ctx);
      if (raw.details) {
        sink.push({
          toolUseId: call.id,
          name: call.name,
          details: raw.details,
        });
      }
      return raw;
    },
  };
}

/**
 * Runs each call through `dispatcher.runTool` in order, stopping before the
 * next call once the signal fires. A call naming an unknown tool still goes
 * through the pipeline, which returns its error result.
 */
export async function runToolCalls(run: {
  dispatcher: HookDispatcher;
  tools: readonly HarnessTool[];
  sessionId: string;
  calls: ToolUseBlock[];
  signal: AbortSignal | undefined;
}): Promise<{
  results: ToolResultBlock[];
  toolResults: ToolResultDetails[];
}> {
  const results: ToolResultBlock[] = [];
  const toolResults: ToolResultDetails[] = [];
  for (const call of run.calls) {
    if (run.signal?.aborted) break;
    const tool = run.tools.find((item) => item.def.name === call.name);
    const recorded = tool && recordingDetails(tool, call, toolResults);
    results.push(
      await run.dispatcher.runTool(call, recorded, {
        sessionId: run.sessionId,
        toolUseId: call.id,
        signal: run.signal,
      }),
    );
  }
  return { results, toolResults };
}
