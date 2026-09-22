/**
 * Apply chunk decisions to a projected message list.
 *
 * Contract: a tool chunk at tier none replaces that tool_result's content
 * with `[omitted]` and leaves isError and the tool_use block in place.
 * Every block is copied. The input messages are not mutated.
 *
 * Intentionally not in the Log: this function. The omitted text is what the
 * request contains; the manifest records the decision, and rebuild reads it
 * back off that entry.
 */

import type { Message } from "@sage/provider/contract";
import type { Chunk } from "./chunks.js";
import type { ChunkDecision } from "./eliminate.js";

export const OMITTED_TEXT = "[omitted]";

export function applyDecisions(
  messages: Message[],
  chunks: Chunk[],
  decisions: ChunkDecision[],
): Message[] {
  const tierById = new Map(
    decisions.map((decision) => [decision.id, decision.tier]),
  );
  const omit = new Set<string>();
  for (const chunk of chunks) {
    if (chunk.kind !== "tool" || chunk.toolUseId === undefined) continue;
    if (tierById.get(chunk.id) === "none") omit.add(chunk.toolUseId);
  }
  return omitToolResults(messages, omit);
}

export function omitToolResults(
  messages: readonly Message[],
  toolUseIds: ReadonlySet<string>,
): Message[] {
  return messages.map((message) => ({
    role: message.role,
    content: message.content.map((block) => {
      if (block.type === "tool_result" && toolUseIds.has(block.toolUseId)) {
        return { ...block, content: OMITTED_TEXT };
      }
      return { ...block };
    }),
  }));
}
