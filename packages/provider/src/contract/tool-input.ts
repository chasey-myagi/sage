/**
 * The replay rule for tool-call arguments, shared by every adapter.
 *
 * Guarantees: a tool_use whose `input` is a JSON object replays as
 * streamed; any other `input` (arguments that did not parse, or parsed to
 * a non-object) replays as an empty object, so one malformed call never
 * makes the next request invalid. `rawInput` on the block is never
 * changed, so the Log keeps the text the model sent.
 *
 * Intentionally not in the Log: the placeholder. It is a pure function of
 * the logged block, so the replayed request is still rebuilt from the Log
 * alone.
 */

import type { ToolUseBlock } from "./message.js";
import { isRecord } from "./record.js";

/** The argument text a tool_use with unusable `input` replays as. */
const EMPTY_TOOL_ARGUMENTS = "{}";

/**
 * The arguments an adapter sends when it replays `block`: `input` for wire
 * formats that carry an object, `text` for those that carry JSON text.
 * Both are the placeholder `{}` when `block.input` is not a JSON object.
 * `text` is `block.rawInput` byte for byte otherwise, or `{}` when that is
 * blank (a call that streamed no arguments). Returns a fresh object for
 * the placeholder; `input` is otherwise `block.input` itself.
 */
export function replayToolInput(block: ToolUseBlock): {
  input: Record<string, unknown>;
  text: string;
} {
  if (!isRecord(block.input)) return { input: {}, text: EMPTY_TOOL_ARGUMENTS };
  const text =
    block.rawInput.trim() === "" ? EMPTY_TOOL_ARGUMENTS : block.rawInput;
  return { input: block.input, text };
}
