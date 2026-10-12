/**
 * Exhaustiveness guard for a `switch` over a closed union.
 *
 * The one definition every layer shares; `@sage/core` re-exports it for
 * packages that only depend on core. Adding a union member makes each
 * switch that ends in `assertNever` fail to compile until it handles it.
 *
 * Intentionally not in the Log: nothing. This is a pure check.
 */

/** Marks an unreachable branch. Throws if reached at run time. */
export function assertNever(value: never): never {
  throw new Error(`Unexpected value: ${String(value)}`);
}
