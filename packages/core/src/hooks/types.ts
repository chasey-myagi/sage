/**
 * Hook envelopes.
 *
 * Contract: observe is read-only and its errors are swallowed. on / guard /
 * around participate in the tool pipeline. Callbacks do not receive the store.
 *
 * Intentionally not in the Log: registrations and observer errors. Observer
 * errors stay on RunSummary.
 */

import type {
  AssistantMessage,
  ToolResultBlock,
  ToolUseBlock,
} from "@sage/provider/contract";
import type { RunSummary, TurnOutcome } from "../session.js";
import type { CompiledView } from "../view/types.js";

export type HookEvents = {
  session_start: { sessionId: string };
  turn_start: { turn: number };
  view_compiled: { view: CompiledView };
  llm_end: { assistant: AssistantMessage };
  tool_pre: { call: ToolUseBlock };
  tool_post: { call: ToolUseBlock; result: ToolResultBlock };
  tool_result: { call: ToolUseBlock; result: ToolResultBlock };
  turn_end: { turn: number; outcome: TurnOutcome };
  session_end: { summary: RunSummary };
};

export type Decision = { deny?: string; updatedInput?: unknown } | undefined;

export type ObserverError = { event: string; error: string };

export interface Hooks {
  observe<E extends keyof HookEvents>(
    event: E,
    fn: (payload: Readonly<HookEvents[E]>) => void | Promise<void>,
  ): () => void;
  on(
    event: "tool_pre",
    fn: (payload: HookEvents["tool_pre"]) => Decision | Promise<Decision>,
  ): () => void;
  on(
    event: "tool_post",
    fn: (
      payload: HookEvents["tool_post"],
    ) => ToolResultBlock | undefined | Promise<ToolResultBlock | undefined>,
  ): () => void;
  guard(fn: (call: ToolUseBlock) => string | undefined): () => void;
  around(
    fn: (
      call: ToolUseBlock,
      next: () => Promise<ToolResultBlock>,
    ) => Promise<ToolResultBlock>,
  ): () => void;
}
