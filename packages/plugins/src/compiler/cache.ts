/**
 * Cache-hit series read back from compile manifests.
 *
 * Contract: each manifest that carries `cache.expectedReadTokens` and
 * `cache.measuredReadTokens` contributes one row, in path order. `ratio` is
 * measured/expected, or 0 when expected is 0. `turn` is the manifest turn.
 *
 * Intentionally not in the Log: this function. The numbers it reads are
 * already on the manifest entries.
 */

import type { StoredEntry } from "@sage/core";

/** One cache-hit row per manifest that recorded both cache numbers. */
export function cacheHitRatio(
  path: readonly StoredEntry[],
): { turn: number; expected: number; measured: number; ratio: number }[] {
  const rows: {
    turn: number;
    expected: number;
    measured: number;
    ratio: number;
  }[] = [];
  for (const entry of path) {
    if (entry.kind !== "manifest" || !isRecord(entry.payload)) continue;
    const cache = entry.payload.cache;
    if (!isRecord(cache)) continue;
    const turn = finite(entry.payload.turn);
    const expected = finite(cache.expectedReadTokens);
    const measured = finite(cache.measuredReadTokens);
    if (
      turn === undefined ||
      expected === undefined ||
      measured === undefined
    ) {
      continue;
    }
    rows.push({
      turn,
      expected,
      measured,
      ratio: expected === 0 ? 0 : measured / expected,
    });
  }
  return rows;
}

function finite(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
