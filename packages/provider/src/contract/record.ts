/**
 * Type guards for untrusted JSON values.
 *
 * The one definition of "is this a non-null, non-array object" and of "is
 * this a finite number". Core and the adapters narrow untrusted JSON (Log
 * lines, wire payloads, tool input) with them; `@sage/core` re-exports them
 * for packages that only depend on core.
 *
 * Intentionally not in the Log: nothing here allocates or records a fact.
 */

/**
 * True for a non-null object that is not an array. Class instances and
 * `Object.create(null)` objects also pass; callers only use it on parsed
 * JSON, where those do not occur.
 */
export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** True for a number that is neither NaN nor infinite. */
export function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

/** `value` when it is a finite number, else `fallback`. */
export function finiteOr<T>(value: unknown, fallback: T): number | T {
  return isFiniteNumber(value) ? value : fallback;
}
