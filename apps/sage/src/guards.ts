/**
 * Exhaustiveness guard for the CLI's closed unions. Value guards for
 * untrusted input (`isRecord`, `isFiniteNumber`) come from `@sage/core`.
 *
 * Intentionally not in the Log: nothing. This is a pure check.
 */

/** Exhaustiveness check for a `switch` over a closed union. Throws if reached. */
export function assertNever(value: never): never {
  throw new Error(`Unexpected value: ${String(value)}`);
}
