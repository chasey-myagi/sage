/**
 * Stream chunks every adapter emits.
 *
 * Guarantees, checked by the shared contract suite:
 * - at most one response chunk, carrying the provider's response id and
 *   model as sent on the wire; it precedes every block_start. A call that
 *   fails before the body begins has none
 * - a usage chunk appears before finish
 * - finish is the last chunk
 * - for each index, block_start precedes that index's deltas, and no delta
 *   for that index appears after its block_end
 * - finish.error.transport is true exactly when the call failed below HTTP:
 *   fetch threw, or reading a response body threw (the SSE body, or the
 *   body of a non-2xx response). Such a failure carries no status. HTTP
 *   statuses whose body was read, error
 *   events inside the stream, and a body that closes without a stop leave
 *   it unset. Callers classify transport failures by this flag alone.
 *
 * Intentionally not in the Log: chunks, the live decode of one call. The
 * assistant entry that lands in the Log is the collected message. Wire SSE
 * frames and the abort signal stay in the adapter.
 */

import type { ContentBlock } from "./message.js";
import type { Usage } from "./usage.js";

/**
 * Why a call ended. `error` carries a `FinishError`; `aborted` means the
 * caller's signal fired.
 */
export type FinishKind =
  | "stop"
  | "tool_use"
  | "max_tokens"
  | "error"
  | "aborted";

/** One decoded stream event. See the invariants above for ordering. */
export type StreamChunk =
  | {
      type: "block_start";
      index: number;
      blockType: ContentBlock["type"];
      id?: string;
      name?: string;
    }
  | { type: "response"; id: string; model: string }
  | { type: "text_delta"; index: number; text: string }
  | { type: "thinking_delta"; index: number; text: string }
  | { type: "tool_input_delta"; index: number; partialJson: string }
  | { type: "block_end"; index: number }
  | { type: "usage"; usage: Usage }
  | {
      type: "finish";
      kind: FinishKind;
      rawFinishReason?: string;
      error?: FinishError;
    };

/**
 * Finish error message when a call ends without a stop: the body closed
 * early, there was no body, or a thrown value carried no message.
 */
export const INCOMPLETE_STREAM = "stream ended before completion";

/** What went wrong in a failed call. `raw` is the provider payload, if any. */
export type FinishError = {
  message: string;
  status?: number;
  raw?: unknown;
  /** Set only for failures below HTTP. See the invariant above. */
  transport?: boolean;
};
