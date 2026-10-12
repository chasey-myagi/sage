/**
 * Which bench rows are faults: tasks lost to the harness or the upstream
 * rather than to the agent.
 *
 * Contract: pure functions of one result row. A row is a fault only when
 * its `status` is `error` and its `reason` is exactly a
 * {@link BenchErrorReason} classified as a fault here; any other reason
 * string, including a hand-edited one, is not a fault.
 *
 * Intentionally not in the Log: the classification. It is derived from
 * `results.json` each time a report is printed.
 */

import { assertNever } from "../guards.js";
import {
  BENCH_ERROR_REASONS,
  type BenchErrorReason,
  type BenchTaskResult,
} from "./types.js";

/** True when `row` is an `error` row whose reason {@link isFaultReason} accepts. */
export function isFaultRow(row: BenchTaskResult): boolean {
  if (row.status !== "error") return false;
  const reason = benchErrorReason(row.reason);
  return reason !== undefined && isFaultReason(reason);
}

/**
 * True for reasons that say nothing about the agent: the task never reached
 * it (`clone-failed`, `clone-unclean`, `setup-failed`, `sage-failed`), its
 * session ended on a provider or harness error or an interrupt
 * (`session-error`, `session-aborted`), or its work could not be read
 * (`diff-failed`). Kept as agent outcomes: `tests-failed`;
 * `test-patch-failed`, because the agent's own edits can make the hidden
 * test patch conflict; and `timeout`, because a slow or looping agent is a
 * policy effect a comparison should see.
 */
function isFaultReason(reason: BenchErrorReason): boolean {
  switch (reason) {
    case "clone-failed":
    case "clone-unclean":
    case "setup-failed":
    case "sage-failed":
    case "session-error":
    case "session-aborted":
    case "diff-failed":
      return true;
    case "timeout":
    case "test-patch-failed":
    case "tests-failed":
      return false;
    default:
      return assertNever(reason);
  }
}

function benchErrorReason(
  value: string | undefined,
): BenchErrorReason | undefined {
  return BENCH_ERROR_REASONS.find((reason) => reason === value);
}
