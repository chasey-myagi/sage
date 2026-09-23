/**
 * Apply chunk decisions to a projected message list.
 *
 * Contract: a tool chunk at tier none replaces that tool_result's content
 * with `renderStub(chunk)` and leaves isError and the tool_use block in
 * place. A tool chunk at tier summary replaces that content with
 * `renderStub(chunk)`, a newline, and
 * `summary (model notes, not evidence): ` plus the decision's summary.
 * Every block is copied. The input messages are not mutated.
 *
 * Intentionally not in the Log: this function. The stub is what the request
 * contains; the manifest records the decision, and rebuild reads it back
 * off that entry.
 */

import type { Message } from "@sage/provider/contract";
import type { Chunk } from "./chunks.js";
import type { ChunkDecision } from "./eliminate.js";
import { renderStub } from "./stub.js";

/** Tool_result text for a tier-summary chunk: stub, newline, labelled note. */
export function renderSummary(chunk: Chunk, summary: string): string {
  return `${renderStub(chunk)}\nsummary (model notes, not evidence): ${summary}`;
}

/**
 * Returns a copy of `messages` with each none or summary tool chunk's
 * tool_result content replaced. Chunks without a decision are unchanged.
 */
export function applyDecisions(
  messages: readonly Message[],
  chunks: Chunk[],
  decisions: ChunkDecision[],
): Message[] {
  const byId = new Map(decisions.map((decision) => [decision.id, decision]));
  const stubs = new Map<string, string>();
  for (const chunk of chunks) {
    if (chunk.kind !== "tool" || chunk.toolUseId === undefined) continue;
    const decision = byId.get(chunk.id);
    if (!decision) continue;
    if (decision.tier === "none") {
      stubs.set(chunk.toolUseId, renderStub(chunk));
      continue;
    }
    if (decision.tier === "summary") {
      stubs.set(chunk.toolUseId, renderSummary(chunk, decision.summary ?? ""));
    }
  }
  return replaceToolResults(messages, stubs);
}

/**
 * Returns a copy of `messages` with each tool_result whose toolUseId is in
 * the map given that content. Every block is shallow-copied.
 */
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
