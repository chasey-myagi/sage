/**
 * Hook envelopes.
 *
 * Guarantees: observers are read-only and their errors are swallowed.
 * `on` / `guard` / `around` take part in the tool pipeline. Callbacks never
 * receive the store.
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

/** Event name → payload, in the order a turn emits them. */
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

/**
 * A `tool_pre` verdict. `deny` refuses the call with that reason (final);
 * `updatedInput` replaces the input later hooks and the tool see.
 * `undefined` lets the call through unchanged.
 */
export type Decision = { deny?: string; updatedInput?: unknown } | undefined;

/** One observer throw, recorded on `RunSummary.observerErrors`. */
export type ObserverError = { event: string; error: string };

/** Registration surface. Every method returns a function that unregisters. */
export interface Hooks {
  /** Read-only listener; awaited in registration order, throws recorded. */
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
  /** Synchronous veto after `tool_pre`: a returned string denies the call. */
  guard(fn: (call: ToolUseBlock) => string | undefined): () => void;
  /**
   * Wraps tool execution; the first registered is outermost. A throw
   * becomes an error tool_result.
   */
  around(
    fn: (
      call: ToolUseBlock,
      next: () => Promise<ToolResultBlock>,
    ) => Promise<ToolResultBlock>,
  ): () => void;
}
