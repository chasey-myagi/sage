/**
 * Type guards for the CLI's untrusted inputs: session jsonl payloads,
 * `results.json`, and task files. `isRecord` comes from `@sage/core`.
 *
 * Intentionally not in the Log: nothing. These are pure predicates.
 */

/** True for a number that is neither NaN nor infinite. */
export function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

/** Exhaustiveness check for a `switch` over a closed union. Throws if reached. */
export function assertNever(value: never): never {
  throw new Error(`Unexpected value: ${String(value)}`);
}
