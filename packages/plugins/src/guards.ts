/**
 * Guards shared across the plugins package for values read back off the
 * Log, where types are not guaranteed.
 *
 * Contract: pure functions; they never throw.
 *
 * Intentionally not in the Log: these helpers. They only read values.
 */

/** `value` when it is a finite number, else undefined. */
export function finite(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}
