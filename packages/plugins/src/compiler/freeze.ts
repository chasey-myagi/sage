/**
 * Frozen chunk decisions for one view epoch.
 *
 * Contract: the freeze record is the first manifest after the `view_epoch`
 * that opened `epoch`. Epoch 0 with no `view_epoch` uses the first manifest
 * before any later `view_epoch`. No such manifest means this is the epoch's
 * first turn, and `frozenDecisions` returns undefined. `frozenSeqCeiling` is
 * the highest message seq before that manifest. Later messages are new.
 *
 * `currentEpoch` is the latest `view_epoch` on the path, or 0 when the path
 * has none. A subtask recompile reads the freeze from that epoch on the
 * parent path.
 *
 * Intentionally not in the Log: the returned map. The manifest entry is the
 * record the next turn reads back.
 */

import type { StoredEntry } from "@sage/core";
import type { Tier } from "./claim.js";
import type { ChunkDecision } from "./eliminate.js";

const TIERS: readonly Tier[] = ["none", "line", "summary", "full"];

export function frozenDecisions(
  path: readonly StoredEntry[],
  epoch: number,
): Map<string, ChunkDecision> | undefined {
  const found = firstManifest(path, epoch);
  if (!found) return undefined;
  return chunkMap(found.entry);
}

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

export function currentEpoch(path: readonly StoredEntry[]): number {
  let epoch = 0;
  for (const entry of path) {
    if (entry.kind !== "view_epoch" || !isRecord(entry.payload)) continue;
    const value = entry.payload.epoch;
    if (typeof value === "number" && Number.isFinite(value)) epoch = value;
  }
  return epoch;
}

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
  let start = -1;
  let opened = false;
  for (let index = 0; index < path.length; index += 1) {
    const entry = path[index];
    if (entry?.kind !== "view_epoch" || !isRecord(entry.payload)) {
      continue;
    }
    if (entry.payload.epoch !== epoch) continue;
    start = index;
    opened = true;
    break;
  }
  if (!opened && epoch !== 0) return undefined;
  for (let index = start + 1; index < path.length; index += 1) {
    const entry = path[index];
    if (!entry) continue;
    if (entry.kind === "view_epoch") break;
    if (entry.kind === "manifest") return { index, entry };
  }
  return undefined;
}

function chunkMap(entry: StoredEntry): Map<string, ChunkDecision> {
  const map = new Map<string, ChunkDecision>();
  if (!isRecord(entry.payload) || !Array.isArray(entry.payload.chunks)) {
    return map;
  }
  for (const chunk of entry.payload.chunks) {
    if (!isRecord(chunk) || typeof chunk.id !== "string") continue;
    if (!isTier(chunk.tier)) continue;
    map.set(chunk.id, {
      id: chunk.id,
      tier: chunk.tier,
      reason: typeof chunk.reason === "string" ? chunk.reason : "kept",
    });
  }
  return map;
}

function isTier(value: unknown): value is Tier {
  return typeof value === "string" && TIERS.includes(value as Tier);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
