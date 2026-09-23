/**
 * Guards shared across the plugins package for values read back off the
 * Log, where types are not guaranteed.
 *
 * Contract: pure functions; they never throw.
 *
 * Intentionally not in the Log: these helpers. They only read values.
 */

import type { Tier } from "@sage/core";

const TIERS: ReadonlySet<unknown> = new Set<Tier>([
  "none",
  "line",
  "summary",
  "full",
]);

/** Whether `value` is one of the four tier names. */
export function isTier(value: unknown): value is Tier {
  return TIERS.has(value);
}

/** `value` when it is a finite number, else undefined. */
export function finite(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}
