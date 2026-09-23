/**
 * Reads `view_epoch` numbers back off a Log path.
 *
 * Contract: an epoch is the payload's finite `epoch` number, taken in path
 * order. Entries of other kinds, and `view_epoch` payloads without a finite
 * `epoch`, are skipped. The store validators already reject those, so the
 * skip only matters for hand-built paths.
 *
 * Intentionally not in the Log: these are reads; nothing here appends.
 */

import { isRecord } from "@sage/provider/contract";
import type { StoredEntry } from "./types.js";

/** Every `view_epoch.epoch` on `path`, in path order (not sorted). */
export function viewEpochs(path: readonly StoredEntry[]): number[] {
  const epochs: number[] = [];
  for (const entry of path) {
    if (entry.kind !== "view_epoch" || !isRecord(entry.payload)) continue;
    const epoch = entry.payload.epoch;
    if (typeof epoch === "number" && Number.isFinite(epoch)) epochs.push(epoch);
  }
  return epochs;
}

/**
 * The epoch of the last `view_epoch` on `path`, or 0 when there is none.
 * "Last" is path order, not the largest value.
 */
export function latestViewEpoch(path: readonly StoredEntry[]): number {
  return viewEpochs(path).at(-1) ?? 0;
}
