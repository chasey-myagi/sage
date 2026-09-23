/**
 * Frozen chunk decisions for one view epoch.
 *
 * Contract: decisions accumulate across every manifest in the epoch, starting
 * at that epoch's `view_epoch`. Epoch 0 with no `view_epoch` starts at the
 * first manifest and stops at the next `view_epoch`. Chunks merge in
 * appearance order. A later manifest does not override an id already
 * recorded; the first tier is the one that stays frozen. A string
 * `summary` on that first chunk is copied with the decision so a later
 * turn can render the same note. No manifest means
 * this is the epoch's first turn, and `frozenDecisions` returns undefined.
 * `frozenSeqCeiling` is the highest message seq before the epoch's first
 * manifest. Later messages are new.
 *
 * A subtask recompile reads the freeze from the parent path's latest
 * epoch (`latestViewEpoch` from `@sage/core`).
 *
 * Intentionally not in the Log: the returned map. The manifest entry is the
 * record the next turn reads back.
 */

import { isRecord, manifestsInEpoch, type StoredEntry } from "@sage/core";
import type { Tier } from "./claim.js";
import type { ChunkDecision } from "./eliminate.js";

const TIERS: readonly Tier[] = ["none", "line", "summary", "full"];

/**
 * The epoch's frozen chunk decisions, first tier per id, or undefined on
 * the epoch's first turn.
 */
export function frozenDecisions(
  path: readonly StoredEntry[],
  epoch: number,
): Map<string, ChunkDecision> | undefined {
  const manifests = manifestsInEpoch(path, epoch);
  if (manifests.length === 0) return undefined;
  const map = new Map<string, ChunkDecision>();
  for (const entry of manifests) mergeFirst(map, entry);
  return map;
}

/**
 * Highest message seq before the epoch's first manifest, or undefined
 * when there is none. Messages above it are new to the epoch.
 */
export function frozenSeqCeiling(
  path: readonly StoredEntry[],
  epoch: number,
): number | undefined {
  const found = firstManifest(path, epoch);
  if (!found) return undefined;
  let max = -1;
  for (let index = 0; index < found.index; index += 1) {
    const entry = path[index];
    if (entry?.kind !== "message") continue;
    if (entry.seq > max) max = entry.seq;
  }
  return max < 0 ? undefined : max;
}

/** The `reason` of the `view_epoch` entry that opened `epoch`, if any. */
export function epochOpenReason(
  path: readonly StoredEntry[],
  epoch: number,
): string | undefined {
  for (const entry of path) {
    if (entry.kind !== "view_epoch" || !isRecord(entry.payload)) continue;
    if (entry.payload.epoch !== epoch) continue;
    return typeof entry.payload.reason === "string"
      ? entry.payload.reason
      : undefined;
  }
  return undefined;
}

function firstManifest(
  path: readonly StoredEntry[],
  epoch: number,
): { index: number; entry: StoredEntry } | undefined {
  const entry = manifestsInEpoch(path, epoch)[0];
  if (!entry) return undefined;
  const index = path.indexOf(entry);
  if (index < 0) return undefined;
  return { index, entry };
}

function mergeFirst(map: Map<string, ChunkDecision>, entry: StoredEntry): void {
  if (!isRecord(entry.payload) || !Array.isArray(entry.payload.chunks)) return;
  for (const chunk of entry.payload.chunks) {
    if (!isRecord(chunk) || typeof chunk.id !== "string") continue;
    if (!isTier(chunk.tier) || map.has(chunk.id)) continue;
    const decision: ChunkDecision = {
      id: chunk.id,
      tier: chunk.tier,
      reason: typeof chunk.reason === "string" ? chunk.reason : "kept",
    };
    if (typeof chunk.summary === "string") decision.summary = chunk.summary;
    map.set(chunk.id, decision);
  }
}

function isTier(value: unknown): value is Tier {
  return typeof value === "string" && TIERS.includes(value as Tier);
}
