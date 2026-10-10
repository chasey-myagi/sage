/**
 * Reads a tool result's `details.childRuns` (see `ChildRun`) for tests.
 *
 * Guarantees: throws instead of returning `undefined`, so a test that
 * expects one child fails at the read.
 *
 * Intentionally not in the Log: nothing; this only reads a result object.
 */

import { isRecord } from "@sage/provider/contract";

/**
 * The single `details.childRuns` item of a tool result. Throws when the
 * result reports no child run or more than one, so a test reading a
 * one-child tool fails loudly instead of reading `undefined`.
 */
export function soleChildRun(result: {
  details?: Record<string, unknown>;
}): Record<string, unknown> {
  const runs = result.details?.childRuns;
  if (!Array.isArray(runs) || runs.length !== 1) {
    throw new Error("expected exactly one child run");
  }
  const [run] = runs;
  if (!isRecord(run)) throw new Error("child run is not an object");
  return run;
}
