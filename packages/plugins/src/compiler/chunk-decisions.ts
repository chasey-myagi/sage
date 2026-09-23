/**
 * Chunk tier decisions under the cumulative freeze of the current epoch.
 *
 * Contract: a chunk that already has a frozen decision keeps it byte for
 * byte; only chunks new to the epoch go through `eliminate`. Frozen kept
 * tokens are charged before the budget runs, so the budget never demotes a
 * frozen chunk. Nothing here reads or writes the Log; the frozen map comes
 * from `frozenDecisions`.
 *
 * Intentionally not in the Log: the intermediate decisions. Only the rows
 * the compiler builds from them reach the manifest.
 */

import type { Chunk } from "./chunks.js";
import type { ChunkDecision } from "./eliminate.js";
import { eliminate } from "./eliminate.js";
import { reasonKind } from "./reasons.js";
import type { ChunkRow } from "./rows.js";

/** Frozen decisions of the current epoch, keyed by chunk id. */
export type FrozenMap = Map<string, ChunkDecision>;

/**
 * Decides a tier for every chunk, in chunk order, and sums the tokens of
 * chunks that are not none.
 *
 * Without `frozen` (an epoch's first turn, or a from-scratch recompile)
 * this is plain `eliminate`. With it, frozen chunks are copied and
 * `eliminate` sees them with their tokens zeroed, as a read-only reference
 * for supersession, plus the new chunks. Only new chunks keep eliminate's
 * tier: recent or full on their first turn, unless the oldest-first budget
 * drops them, and that drop is frozen too. Recent slots are then reclaimed
 * past entry-scored chunks (see `reclaimRecentSlots`).
 */
export function decideChunks(
  chunks: Chunk[],
  frozen: FrozenMap | undefined,
  keepRecent: number,
  remainingBudget: number,
): { decisions: ChunkDecision[]; used: number } {
  if (!frozen) {
    const eliminated = eliminate(chunks, { keepRecent, remainingBudget });
    return { decisions: eliminated.decisions, used: eliminated.used };
  }
  let frozenKept = 0;
  for (const chunk of chunks) {
    const kept = frozen.get(chunk.id);
    if (!kept || kept.tier === "none") continue;
    frozenKept += chunk.tokens;
  }
  const input = chunks.map((chunk) =>
    frozen.has(chunk.id) ? { ...chunk, tokens: 0 } : chunk,
  );
  const eliminated = eliminate(input, {
    keepRecent,
    remainingBudget: Math.max(0, remainingBudget - frozenKept),
  });
  const decisions = chunks.map((chunk, index) => {
    const kept = frozen.get(chunk.id);
    if (kept) return copyFrozen(kept);
    return (
      eliminated.decisions[index] ?? {
        id: chunk.id,
        tier: "full" as const,
        reason: "kept",
      }
    );
  });
  reclaimRecentSlots(chunks, frozen, decisions, keepRecent);
  let used = 0;
  for (let index = 0; index < chunks.length; index += 1) {
    if (decisions[index]?.tier === "none") continue;
    used += chunks[index]?.tokens ?? 0;
  }
  return { decisions, used };
}

function copyFrozen(kept: ChunkDecision): ChunkDecision {
  return {
    id: kept.id,
    tier: kept.tier,
    reason: kept.reason,
    ...(kept.summary !== undefined ? { summary: kept.summary } : {}),
  };
}

/**
 * An entry-scored chunk does not consume a `keepRecent` slot, and
 * `keepRecent` does not force it back to full. When the frozen map holds
 * entry-scored chunks, the recent window is recounted over the other tool
 * chunks; a new chunk inside that window whose reason is not already
 * recent, superseded, or retry becomes full with reason `recent`.
 * Mutates `decisions` in place.
 */
function reclaimRecentSlots(
  chunks: readonly Chunk[],
  frozen: FrozenMap,
  decisions: ChunkDecision[],
  keepRecent: number,
): void {
  if (keepRecent <= 0) return;
  const entryScored = new Set<string>();
  for (const [id, decision] of frozen) {
    if (reasonKind(decision.reason) === "entry-scorer") entryScored.add(id);
  }
  if (entryScored.size === 0) return;
  const slotIndexes: number[] = [];
  for (let index = 0; index < chunks.length; index += 1) {
    const chunk = chunks[index];
    if (chunk?.kind !== "tool" || entryScored.has(chunk.id)) continue;
    slotIndexes.push(index);
  }
  const recent = new Set(slotIndexes.slice(-keepRecent));
  for (const index of recent) {
    const chunk = chunks[index];
    const decision = decisions[index];
    if (!chunk || !decision || frozen.has(chunk.id)) continue;
    const kind = reasonKind(decision.reason);
    if (kind === "recent" || kind === "superseded" || kind === "retry") {
      continue;
    }
    decisions[index] = { id: chunk.id, tier: "full", reason: "recent" };
  }
}

/**
 * Freeze accounting for the manifest `budget`. `frozenTokens` sums the
 * tokens of frozen rows that are not none. `staleOmittable` is the part of
 * that sum a from-scratch recompile would set to none. Both describe the
 * frozen view even on a rebuild turn, and both are 0 without a freeze.
 */
export function freezeAccounting(
  frozen: FrozenMap | undefined,
  frozenRows: readonly ChunkRow[],
  recompiledRows: readonly ChunkRow[],
): { frozenTokens: number; staleOmittable: number } {
  if (!frozen) return { frozenTokens: 0, staleOmittable: 0 };
  const recompiledById = new Map(recompiledRows.map((row) => [row.id, row]));
  let frozenTokens = 0;
  let staleOmittable = 0;
  for (const row of frozenRows) {
    if (!frozen.has(row.id) || row.tier === "none") continue;
    frozenTokens += row.tokens;
    if (recompiledById.get(row.id)?.tier === "none") {
      staleOmittable += row.tokens;
    }
  }
  return { frozenTokens, staleOmittable };
}
