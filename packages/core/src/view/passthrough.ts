/**
 * Passthrough view. Projects message entries in path order.
 *
 * Contract: the kernel's default ViewProvider. It does not compress. A dangling
 * tool_use left by a crash gets a synthesized TOOL_OUTCOME_UNKNOWN result and
 * an anomaly. An orphan tool_result is dropped and recorded. An empty message
 * left by that drop is dropped and recorded. The head is the one the session
 * injected on the context. Epoch is the latest view_epoch on the path, or 0.
 * Every emitted chunk is tier "full" with reason "passthrough".
 * cacheBreakpoints is ["head"].
 *
 * Intentionally not in the Log: the repaired messages and the anomaly list.
 * Callers append a synthesized result themselves if they want it in history.
 */

import type { ContentBlock, Message } from "@sage/provider/contract";
import { synthesizeToolResult } from "../log/repair.js";
import type { StoredEntry } from "../log/types.js";
import type {
  Anomaly,
  ChunkRecord,
  CompiledView,
  ViewContext,
  ViewProvider,
} from "./types.js";

function latestEpoch(path: readonly StoredEntry[]): number {
  let epoch = 0;
  for (const entry of path) {
    if (entry.kind !== "view_epoch") continue;
    const value = (entry.payload as { epoch?: unknown }).epoch;
    if (typeof value === "number") epoch = value;
  }
  return epoch;
}

function isMessage(payload: unknown): payload is Message {
  if (
    typeof payload !== "object" ||
    payload === null ||
    Array.isArray(payload)
  ) {
    return false;
  }
  const record = payload as { role?: unknown; content?: unknown };
  return (
    (record.role === "user" || record.role === "assistant") &&
    Array.isArray(record.content)
  );
}

function project(path: readonly StoredEntry[]): {
  messages: Message[];
  chunks: ChunkRecord[];
  anomalies: Anomaly[];
} {
  const messages: Message[] = [];
  const chunks: ChunkRecord[] = [];
  const anomalies: Anomaly[] = [];
  const open: string[] = [];

  const flush = (id: string) => {
    if (open.length === 0) return;
    const content = open.map((toolUseId) => {
      anomalies.push({ kind: "dangling_tool_use", id: toolUseId });
      return synthesizeToolResult(toolUseId, "TOOL_OUTCOME_UNKNOWN");
    });
    open.length = 0;
    messages.push({ role: "user", content });
    chunks.push({ id, tier: "full", reason: "passthrough" });
  };

  for (const entry of path) {
    if (entry.kind !== "message" || !isMessage(entry.payload)) continue;
    if (entry.payload.role === "assistant" && open.length > 0) {
      flush(`dangling-before:${entry.id}`);
    }
    const content: ContentBlock[] = [];
    const opened: string[] = [];
    for (const block of entry.payload.content) {
      if (block.type === "tool_result") {
        const index = open.indexOf(block.toolUseId);
        if (index === -1) {
          anomalies.push({ kind: "orphan_tool_result", id: block.toolUseId });
          continue;
        }
        open.splice(index, 1);
        content.push(block);
        continue;
      }
      content.push(block);
      if (block.type === "tool_use") opened.push(block.id);
    }
    if (content.length === 0) {
      anomalies.push({ kind: "empty_message" });
      continue;
    }
    messages.push({ role: entry.payload.role, content });
    chunks.push({ id: entry.id, tier: "full", reason: "passthrough" });
    open.push(...opened);
  }
  if (open.length > 0) flush(`dangling:${open[0] ?? "end"}`);
  return { messages, chunks, anomalies };
}

export const passthroughView: ViewProvider = {
  view(ctx: ViewContext): CompiledView {
    const epoch = latestEpoch(ctx.path);
    const projected = project(ctx.path);
    return {
      epoch,
      head: ctx.head,
      messages: projected.messages,
      cacheBreakpoints: ["head"],
      manifest: {
        epoch,
        turn: ctx.turn,
        claims: [],
        chunks: projected.chunks,
        anomalies: projected.anomalies,
      },
    };
  },
};
