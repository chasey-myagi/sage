/**
 * Apply chunk decisions to a projected message list.
 *
 * Contract: a tool chunk at tier none replaces that tool_result's content
 * with `renderStub(chunk)` and leaves isError and the tool_use block in
 * place. Every block is copied. The input messages are not mutated.
 *
 * Intentionally not in the Log: this function. The stub is what the request
 * contains; the manifest records the decision, and rebuild reads it back
 * off that entry.
 */

import type { Message } from "@sage/provider/contract";
import type { Chunk } from "./chunks.js";
import type { ChunkDecision } from "./eliminate.js";
import { renderStub } from "./stub.js";

export function applyDecisions(
  messages: Message[],
  chunks: Chunk[],
  decisions: ChunkDecision[],
): Message[] {
  const tierById = new Map(
    decisions.map((decision) => [decision.id, decision.tier]),
  );
  const stubs = new Map<string, string>();
  for (const chunk of chunks) {
    if (chunk.kind !== "tool" || chunk.toolUseId === undefined) continue;
    if (tierById.get(chunk.id) !== "none") continue;
    stubs.set(chunk.toolUseId, renderStub(chunk));
  }
  return replaceToolResults(messages, stubs);
}

export function replaceToolResults(
  messages: readonly Message[],
  contentByToolUseId: ReadonlyMap<string, string>,
): Message[] {
  return messages.map((message) => ({
    role: message.role,
    content: message.content.map((block) => {
      if (block.type !== "tool_result") return { ...block };
      const content = contentByToolUseId.get(block.toolUseId);
      if (content === undefined) return { ...block };
      return { ...block, content };
    }),
  }));
}
