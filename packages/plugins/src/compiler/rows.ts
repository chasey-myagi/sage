/**
 * Manifest rows the compiler writes for chunks and claims, and the builders
 * that produce them.
 *
 * Contract: every chunk row is built here, so the key order that ends up in
 * the Log is fixed: `id`, `tier`, `reason`, `tokens`, then `originalTokens`
 * and `summary` when present. A tool chunk at tier none keeps its tool_use
 * and its tool_result content becomes the one-line stub; `tokens` then
 * counts the stub and `originalTokens` keeps the pre-stub estimate. A tool
 * chunk at tier summary counts the stub plus the note the same way.
 *
 * Intentionally not in the Log: the builders. The rows themselves are
 * copied verbatim onto the manifest entry.
 */

import type { ClaimRecord } from "@sage/core";
import { renderSummary } from "./apply.js";
import type { TokenCounter } from "./budget.js";
import type { Chunk } from "./chunks.js";
import type { Placement, Tier } from "./claim.js";
import type { ChunkDecision } from "./eliminate.js";
import { renderStub } from "./stub.js";

/** A claim's line on the manifest: the allocation plus rendering facts. */
export type ManifestClaim = ClaimRecord & {
  /** sha256 of the rendered claim text; absent at tier none. */
  contentHash?: string;
  placement?: Placement;
  priority?: number;
};

/** A chunk's line on the manifest. */
export type ChunkRow = {
  id: string;
  tier: Tier;
  reason: string;
  /** Tokens the chunk occupies in the view at `tier`. */
  tokens: number;
  /** Tokens before stubbing; set only when a stub replaced the result. */
  originalTokens?: number;
  /** The scorer's note; set only at tier summary. */
  summary?: string;
};

/** Row for a chunk shown as-is; `tokens` is the chunk's own estimate. */
export function keptRow(
  id: string,
  tier: Tier,
  reason: string,
  tokens: number,
): ChunkRow {
  return { id, tier, reason, tokens };
}

/** Row for a tool chunk at tier none: the result is replaced by the stub. */
export function stubRow(
  chunk: Chunk,
  reason: string,
  count: TokenCounter,
): ChunkRow {
  return {
    id: chunk.id,
    tier: "none",
    reason,
    tokens: count(renderStub(chunk)),
    originalTokens: chunk.tokens,
  };
}

/** Row for a tool chunk at tier summary: stub, newline, and `summary`. */
export function summaryRow(
  chunk: Chunk,
  reason: string,
  summary: string,
  count: TokenCounter,
): ChunkRow {
  return {
    id: chunk.id,
    tier: "summary",
    reason,
    tokens: count(renderSummary(chunk, summary)),
    originalTokens: chunk.tokens,
    summary,
  };
}

/** Converts rows back into the decisions `applyDecisions` consumes. */
export function rowsToDecisions(rows: readonly ChunkRow[]): ChunkDecision[] {
  return rows.map((row) => ({
    id: row.id,
    tier: row.tier,
    reason: row.reason,
    ...(row.summary !== undefined ? { summary: row.summary } : {}),
  }));
}

/**
 * Indexes items by `id`. The first item with a given id wins, matching a
 * linear `find` over the same list.
 */
export function firstById<T extends { id: string }>(
  items: readonly T[],
): Map<string, T> {
  const byId = new Map<string, T>();
  for (const item of items) {
    if (!byId.has(item.id)) byId.set(item.id, item);
  }
  return byId;
}
