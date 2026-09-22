/**
 * Anchor token estimates on the previous provider usage.
 *
 * Contract: `measuredInput` is the latest assistant message's
 * `details.usage` input total (`inputOther + inputCacheRead +
 * inputCacheCreation`). `calibrationFor` pairs that total with the previous
 * manifest's `budget.used`. `head` is the first computable turn of the
 * current epoch: that turn's measured input minus its `budget.used`,
 * floored at 0. When that turn has no usage, `head` is the most recent
 * computable value on the path. Later turns set `ratio` to
 * `(measured − head) / estimated`. No usage, or a zero estimate, yields
 * undefined; callers then use ratio 1 and head 0. Callers scale block
 * tokens by the raw ratio, including when that ratio is above 1.
 * `expectedFrozenPrefix` reads the previous manifest's own calibration
 * and that manifest's `budget.frozenTokens`: `head + ratio × frozenTokens`.
 * No calibration on that manifest uses head 0 and ratio 1. The product is
 * the frozen-prefix estimate the previous turn emitted, so a later epoch's
 * re-anchored head is not applied to the earlier freeze. A head below 0
 * is stored as 0: a negative head makes new-view tokens negative and the
 * cost check reopens immediately.
 *
 * Intentionally not in the Log: these functions. The manifest stores the
 * resulting calibration object.
 */

import type { StoredEntry } from "@sage/core";
import type { Message } from "@sage/provider/contract";

export type Calibration = {
  measured: number;
  estimated: number;
  head: number;
  ratio: number;
};

export function measuredInput(
  path: readonly StoredEntry[],
): number | undefined {
  const usage = latestUsage(path);
  if (!usage) return undefined;
  return inputTotal(usage);
}

export function measuredCacheRead(path: readonly StoredEntry[]): number {
  const usage = latestUsage(path);
  const read = usage ? finite(usage.inputCacheRead) : undefined;
  return read ?? 0;
}

export function calibrationFor(
  path: readonly StoredEntry[],
): Calibration | undefined {
  const measured = measuredInput(path);
  const estimated = previousEstimate(path);
  if (measured === undefined || estimated === undefined || estimated === 0) {
    return undefined;
  }
  const head = anchorHead(path) ?? 0;
  return { measured, estimated, head, ratio: (measured - head) / estimated };
}

export function expectedFrozenPrefix(path: readonly StoredEntry[]): number {
  const manifest = previousManifest(path);
  if (!manifest || !isRecord(manifest.payload)) return 0;
  const prior = storedCalibration(manifest.payload);
  return prior.head + frozenTokens(manifest.payload) * prior.ratio;
}

function anchorHead(path: readonly StoredEntry[]): number | undefined {
  const epoch = currentEpoch(path);
  const range = epochRange(path, epoch);
  const slice = path.slice(range.start, range.end);
  const first = firstHead(slice);
  if (first !== undefined) return first;
  return latestHead(path.slice(0, range.end));
}

function firstHead(path: readonly StoredEntry[]): number | undefined {
  return headFromPairs(path, false);
}

function latestHead(path: readonly StoredEntry[]): number | undefined {
  return headFromPairs(path, true);
}

function headFromPairs(
  path: readonly StoredEntry[],
  latest: boolean,
): number | undefined {
  let pending: number | undefined;
  let found: number | undefined;
  for (const entry of path) {
    if (entry.kind === "manifest") {
      pending = budgetUsed(entry);
      continue;
    }
    if (entry.kind !== "message" || !isAssistant(entry)) continue;
    const usage = entry.details?.usage;
    if (!isRecord(usage) || pending === undefined || pending === 0) continue;
    const measured = inputTotal(usage);
    if (measured === undefined) continue;
    found = Math.max(0, measured - pending);
    if (!latest) return found;
    pending = undefined;
  }
  return found;
}

function currentEpoch(path: readonly StoredEntry[]): number {
  let epoch = 0;
  for (const entry of path) {
    if (entry.kind !== "view_epoch" || !isRecord(entry.payload)) continue;
    const value = finite(entry.payload.epoch);
    if (value !== undefined) epoch = value;
  }
  return epoch;
}

function epochRange(
  path: readonly StoredEntry[],
  epoch: number,
): { start: number; end: number } {
  let start = 0;
  let opened = epoch === 0;
  for (let index = 0; index < path.length; index += 1) {
    const entry = path[index];
    if (entry?.kind !== "view_epoch" || !isRecord(entry.payload)) continue;
    if (entry.payload.epoch !== epoch) continue;
    start = index;
    opened = true;
    break;
  }
  if (!opened) return { start: path.length, end: path.length };
  let end = path.length;
  for (let index = start + 1; index < path.length; index += 1) {
    if (path[index]?.kind === "view_epoch") {
      end = index;
      break;
    }
  }
  return { start, end };
}

function previousEstimate(path: readonly StoredEntry[]): number | undefined {
  const manifest = previousManifest(path);
  if (!manifest) return undefined;
  return budgetUsed(manifest);
}

function previousManifest(
  path: readonly StoredEntry[],
): StoredEntry | undefined {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const entry = path[index];
    if (entry?.kind === "manifest") return entry;
  }
  return undefined;
}

function budgetUsed(entry: StoredEntry): number | undefined {
  if (!isRecord(entry.payload)) return undefined;
  const budget = entry.payload.budget;
  if (typeof budget === "number") return finite(budget);
  if (!isRecord(budget)) return undefined;
  return finite(budget.used);
}

function frozenTokens(payload: Record<string, unknown>): number {
  const budget = payload.budget;
  if (!isRecord(budget)) return 0;
  return finite(budget.frozenTokens) ?? 0;
}

function storedCalibration(payload: Record<string, unknown>): {
  head: number;
  ratio: number;
} {
  const calibration = payload.calibration;
  if (!isRecord(calibration)) return { head: 0, ratio: 1 };
  return {
    head: finite(calibration.head) ?? 0,
    ratio: finite(calibration.ratio) ?? 1,
  };
}

function latestUsage(
  path: readonly StoredEntry[],
): Record<string, unknown> | undefined {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const entry = path[index];
    if (!entry || !isAssistant(entry)) continue;
    const usage = entry.details?.usage;
    return isRecord(usage) ? usage : undefined;
  }
  return undefined;
}

function isAssistant(entry: StoredEntry): boolean {
  return (
    entry.kind === "message" &&
    isMessage(entry.payload) &&
    entry.payload.role === "assistant"
  );
}

function inputTotal(usage: Record<string, unknown>): number | undefined {
  const other = finite(usage.inputOther);
  const read = finite(usage.inputCacheRead);
  const created = finite(usage.inputCacheCreation);
  if (other === undefined || read === undefined || created === undefined) {
    return undefined;
  }
  return other + read + created;
}

function finite(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isMessage(value: unknown): value is Message {
  if (!isRecord(value)) return false;
  return (
    (value.role === "user" || value.role === "assistant") &&
    Array.isArray(value.content)
  );
}
