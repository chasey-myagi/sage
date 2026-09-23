/**
 * Type guard for plain JSON objects.
 *
 * The one definition of "is this a non-null, non-array object". Core and the
 * adapters narrow untrusted JSON (Log lines, wire payloads, tool input) with
 * it; `@sage/core` re-exports it for packages that only depend on core.
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
