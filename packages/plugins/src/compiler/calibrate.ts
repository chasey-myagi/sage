/**
 * Anchor token estimates on the previous provider usage.
 *
 * Contract: `measuredInput` is the latest assistant message's
 * `details.usage` input total (`inputOther + inputCacheRead +
 * inputCacheCreation`). `calibrationFor` pairs that total with the previous
 * manifest's `budget.used`. The ratio is measured/estimated. Callers scale
 * later new-view estimates by it. No usage, or a zero estimate, yields
 * undefined.
 *
 * Intentionally not in the Log: these functions. The manifest stores the
 * resulting calibration object.
 */

import type { StoredEntry } from "@sage/core";
import type { Message } from "@sage/provider/contract";

export type Calibration = {
  measured: number;
  estimated: number;
  ratio: number;
};

export function measuredInput(
  path: readonly StoredEntry[],
): number | undefined {
  const usage = latestUsage(path);
  if (!usage) return undefined;
  const other = finite(usage.inputOther);
  const read = finite(usage.inputCacheRead);
  const created = finite(usage.inputCacheCreation);
  if (other === undefined || read === undefined || created === undefined) {
    return undefined;
  }
  return other + read + created;
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
  return { measured, estimated, ratio: measured / estimated };
}

function previousEstimate(path: readonly StoredEntry[]): number | undefined {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const entry = path[index];
    if (entry?.kind !== "manifest" || !isRecord(entry.payload)) continue;
    const budget = entry.payload.budget;
    if (!isRecord(budget)) continue;
    return finite(budget.used);
  }
  return undefined;
}

function latestUsage(
  path: readonly StoredEntry[],
): Record<string, unknown> | undefined {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const entry = path[index];
    if (entry?.kind !== "message" || !isMessage(entry.payload)) {
      continue;
    }
    if (entry.payload.role !== "assistant") continue;
    const usage = entry.details?.usage;
    return isRecord(usage) ? usage : undefined;
  }
  return undefined;
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
