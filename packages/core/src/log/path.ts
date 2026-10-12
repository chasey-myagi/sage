/**
 * Reads facts back off a Log path: view_epoch numbers, the span, manifests,
 * and prefix message of an epoch, and request_header payloads.
 *
 * Guarantees: pure reads in path order. Entries are returned as stored; a
 * message read out of `details` is cloned. A `view_epoch` payload without
 * a finite `epoch` is skipped; the store validators already reject those,
 * so the skip only matters for hand-built paths.
 *
 * Intentionally not in the Log: these are reads; nothing here appends.
 */

import {
  isFiniteNumber,
  isRecord,
  type Message,
} from "@sage/provider/contract";
import type { RequestHeaderPayload } from "./kinds.js";
import type { StoredEntry } from "./types.js";

/** Every `view_epoch.epoch` on `path`, in path order (not sorted). */
export function viewEpochs(path: readonly StoredEntry[]): number[] {
  const epochs: number[] = [];
  for (const entry of path) {
    if (entry.kind !== "view_epoch" || !isRecord(entry.payload)) continue;
    const epoch = entry.payload.epoch;
    if (isFiniteNumber(epoch)) epochs.push(epoch);
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

/**
 * Narrows an untrusted value to a user or assistant `Message`: a `role` of
 * `user` or `assistant` and an array `content`. Blocks are not checked.
 */
export function isMessage(value: unknown): value is Message {
  return (
    isRecord(value) &&
    (value.role === "user" || value.role === "assistant") &&
    Array.isArray(value.content)
  );
}

/**
 * Index range of the entries inside `epoch`: `start` is just after the
 * first `view_epoch` for `epoch` (0 for epoch 0 when no `view_epoch`
 * opened it) and `end` is the next `view_epoch` after that, else
 * `path.length`. Undefined when `epoch` is not 0 and never opened.
 */
export function epochSpan(
  path: readonly StoredEntry[],
  epoch: number,
): { start: number; end: number } | undefined {
  let start = epoch === 0 ? 0 : -1;
  for (let index = 0; index < path.length; index += 1) {
    const entry = path[index];
    if (entry?.kind !== "view_epoch" || !isRecord(entry.payload)) continue;
    if (entry.payload.epoch !== epoch) continue;
    start = index + 1;
    break;
  }
  if (start < 0) return undefined;
  let end = path.length;
  for (let index = start; index < path.length; index += 1) {
    if (path[index]?.kind === "view_epoch") {
      end = index;
      break;
    }
  }
  return { start, end };
}

/**
 * The `manifest` entries inside `epoch` (see {@link epochSpan}), in path
 * order; empty when the epoch was never opened.
 */
export function manifestsInEpoch(
  path: readonly StoredEntry[],
  epoch: number,
): StoredEntry[] {
  const span = epochSpan(path, epoch);
  if (!span) return [];
  return path
    .slice(span.start, span.end)
    .filter((entry) => entry.kind === "manifest");
}

/**
 * A clone of the last `view_epoch.details.prefixMessage` recorded for
 * `epoch`: the first message of the view that opened it. Undefined when
 * that epoch stored none or the stored value is not a message.
 */
export function prefixMessageFor(
  path: readonly StoredEntry[],
  epoch: number,
): Message | undefined {
  let found: Message | undefined;
  for (const entry of path) {
    if (entry.kind !== "view_epoch" || !isRecord(entry.payload)) continue;
    if (entry.payload.epoch !== epoch) continue;
    const stored = entry.details?.prefixMessage;
    if (isMessage(stored)) found = stored;
  }
  return found ? structuredClone(found) : undefined;
}

/** Index of the last request_header on `path`, or -1. */
export function lastHeaderIndex(path: readonly StoredEntry[]): number {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    if (path[index]?.kind === "request_header") return index;
  }
  return -1;
}

/**
 * The payload of `entry` when it is a request_header, else undefined.
 * Narrows by `kind` only: kernel stores ran the request_header validator on
 * append and on load, so the shape is not checked again. A hand-built entry
 * that skipped a store gets no such guarantee.
 */
export function requestHeaderOf(
  entry: StoredEntry,
): RequestHeaderPayload | undefined {
  if (entry.kind !== "request_header") return undefined;
  // The store validator proved this shape before the entry reached a path.
  return entry.payload as RequestHeaderPayload;
}

/** Payload of the last request_header on `path`, or undefined. */
export function lastRequestHeader(
  path: readonly StoredEntry[],
): RequestHeaderPayload | undefined {
  const entry = path[lastHeaderIndex(path)];
  return entry ? requestHeaderOf(entry) : undefined;
}
