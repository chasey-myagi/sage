/**
 * Deterministic tier decisions for history chunks. Does not touch messages.
 *
 * Contract: each chunk takes the first matching reason, in order. The last
 * `keepRecent` tool chunks are tier full, reason `recent` (`keepRecent` <= 0
 * marks none). A tool chunk whose every access is covered by a later
 * non-error tool chunk is tier none, reason `superseded_by:<id>`, where id
 * is the latest covering chunk. A read or search is covered by a later read,
 * write, or search of the same key. A write is covered only by a later write
 * of the same key. Keys compare as strings. An isError tool chunk whose next
 * tool chunk has the same name and deep-equal input is tier none, reason
 * `retry_intermediate`. Then the oldest tool chunk that is still full and not
 * recent drops to none, reason `budget:oldest-first`, until the kept total
 * fits `remainingBudget`. User and assistant prose stay tier full, reason
 * `kept`. `used` sums tokens of chunks that are not none. `overrun` is how
 * far `used` still sits above `remainingBudget`. `summary` is not set here;
 * a scorer may attach it when it chooses tier summary.
 *
 * Intentionally not in the Log: this function. Callers copy decisions onto
 * the manifest entry.
 */

import type { Access } from "@sage/core";
import type { Chunk } from "./chunks.js";
import type { Tier } from "./claim.js";

/** The tier chosen for one chunk and the rule that chose it. */
export type ChunkDecision = {
  id: string;
  tier: Tier;
  reason: string;
  /** Set by a scorer for tier summary. eliminate never writes this. */
  summary?: string;
};

/**
 * Decides a tier for every chunk, in chunk order, by the rules in the file
 * header. Pure; `chunks` is not mutated.
 */
export function eliminate(
  chunks: Chunk[],
  opts: { keepRecent: number; remainingBudget: number },
): { decisions: ChunkDecision[]; used: number; overrun: number } {
  const toolIndexes: number[] = [];
  for (let index = 0; index < chunks.length; index += 1) {
    if (chunks[index]?.kind === "tool") toolIndexes.push(index);
  }
  const recent = new Set<number>(
    opts.keepRecent > 0 ? toolIndexes.slice(-opts.keepRecent) : [],
  );

  const decisions: ChunkDecision[] = chunks.map((chunk, index) => {
    if (recent.has(index)) {
      return { id: chunk.id, tier: "full", reason: "recent" };
    }
    const superseded = supersededBy(chunks, index);
    if (superseded !== undefined) {
      return {
        id: chunk.id,
        tier: "none",
        reason: `superseded_by:${superseded}`,
      };
    }
    if (isRetry(chunks, index)) {
      return { id: chunk.id, tier: "none", reason: "retry_intermediate" };
    }
    return { id: chunk.id, tier: "full", reason: "kept" };
  });

  let used = 0;
  for (let index = 0; index < chunks.length; index += 1) {
    if (decisions[index]?.tier === "none") continue;
    used += chunks[index]?.tokens ?? 0;
  }

  for (
    let index = 0;
    index < chunks.length && used > opts.remainingBudget;
    index += 1
  ) {
    const chunk = chunks[index];
    const decision = decisions[index];
    if (!chunk || !decision) continue;
    if (chunk.kind !== "tool" || recent.has(index)) continue;
    if (decision.tier === "none") continue;
    decisions[index] = {
      id: chunk.id,
      tier: "none",
      reason: "budget:oldest-first",
    };
    used -= chunk.tokens;
  }

  return {
    decisions,
    used,
    overrun: Math.max(0, used - opts.remainingBudget),
  };
}

function supersededBy(
  chunks: readonly Chunk[],
  index: number,
): string | undefined {
  const chunk = chunks[index];
  if (chunk?.kind !== "tool") return undefined;
  if (chunk.accesses.length === 0) return undefined;
  const open = chunk.accesses.map(() => true);
  let latest: string | undefined;
  for (let later = index + 1; later < chunks.length; later += 1) {
    const next = chunks[later];
    if (next?.kind !== "tool") continue;
    if (next.isError === true) continue;
    let touched = false;
    for (let access = 0; access < chunk.accesses.length; access += 1) {
      const earlier = chunk.accesses[access];
      if (!earlier) continue;
      if (!next.accesses.some((item) => covers(earlier, item))) continue;
      open[access] = false;
      touched = true;
    }
    if (touched) latest = next.id;
  }
  if (latest === undefined || open.some((pending) => pending)) return undefined;
  return latest;
}

function covers(earlier: Access, later: Access): boolean {
  if (earlier.key !== later.key) return false;
  if (earlier.mode === "write") return later.mode === "write";
  return (
    later.mode === "read" || later.mode === "write" || later.mode === "search"
  );
}

function isRetry(chunks: readonly Chunk[], index: number): boolean {
  const chunk = chunks[index];
  if (chunk?.kind !== "tool" || chunk.isError !== true) return false;
  for (let later = index + 1; later < chunks.length; later += 1) {
    const next = chunks[later];
    if (next?.kind !== "tool") continue;
    return (
      next.toolName === chunk.toolName && deepEqual(next.input, chunk.input)
    );
  }
  return false;
}

function deepEqual(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true;
  if (typeof left !== "object" || typeof right !== "object") return false;
  if (left === null || right === null) return false;
  if (Array.isArray(left) || Array.isArray(right)) {
    if (!Array.isArray(left) || !Array.isArray(right)) return false;
    if (left.length !== right.length) return false;
    for (let index = 0; index < left.length; index += 1) {
      if (!deepEqual(left[index], right[index])) return false;
    }
    return true;
  }
  const leftRecord = left as Record<string, unknown>;
  const rightRecord = right as Record<string, unknown>;
  const keys = Object.keys(leftRecord);
  if (keys.length !== Object.keys(rightRecord).length) return false;
  for (const key of keys) {
    if (!Object.hasOwn(rightRecord, key)) return false;
    if (!deepEqual(leftRecord[key], rightRecord[key])) return false;
  }
  return true;
}
