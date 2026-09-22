/**
 * Stream vocabulary shared by every adapter.
 *
 * Invariants, guarded by the shared contract suite:
 * - a usage chunk appears before finish
 * - finish is the last chunk
 * - for each index, block_start precedes that index's deltas, and no delta
 *   for that index appears after its block_end
 *
 * Deliberately not a Log entry: chunks are the live decode of one call.
 * The assistant entry that lands in the Log is the collected message (#4).
 * Wire SSE frames and the abort signal stay in the adapter.
 */

import type { ContentBlock } from "./message.js";
import type { Usage } from "./usage.js";

export type FinishKind =
  | "stop"
  | "tool_use"
  | "max_tokens"
  | "error"
  | "aborted";

export type StreamChunk =
  | {
      type: "block_start";
      index: number;
      blockType: ContentBlock["type"];
      id?: string;
      name?: string;
    }
  | { type: "text_delta"; index: number; text: string }
  | { type: "thinking_delta"; index: number; text: string }
  | { type: "tool_input_delta"; index: number; partialJson: string }
  | { type: "block_end"; index: number }
  | { type: "usage"; usage: Usage }
  | {
      type: "finish";
      kind: FinishKind;
      rawFinishReason?: string;
      error?: { message: string; status?: number; raw?: unknown };
    };
