/**
 * Reads facts back off a Log path: view_epoch numbers and the last
 * request_header.
 *
 * Guarantees: pure reads in path order. A `view_epoch` payload without a
 * finite `epoch` is skipped; the store validators already reject those, so
 * the skip only matters for hand-built paths.
 *
 * Intentionally not in the Log: these are reads; nothing here appends.
 */

import { isRecord } from "@sage/provider/contract";
import type { RequestHeaderPayload } from "./kinds.js";
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

/** Index of the last request_header on `path`, or -1. */
export function lastHeaderIndex(path: readonly StoredEntry[]): number {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    if (path[index]?.kind === "request_header") return index;
  }
  return -1;
}

/** Payload of the last request_header on `path`, or undefined. */
export function lastRequestHeader(
  path: readonly StoredEntry[],
): RequestHeaderPayload | undefined {
  const payload = path[lastHeaderIndex(path)]?.payload;
  // The request_header validator already ran on append and on load.
  return isRecord(payload) ? (payload as RequestHeaderPayload) : undefined;
}
