/**
 * Turns one chunk's triage answers into a tier. Jev judges; this code
 * owns the policy.
 *
 * Guarantees: full for an `isError` chunk without asking; full when a
 * failure report or a new location reaches `full`; none when the failure,
 * the location, and every `keep_<i>` answer are below `low`; otherwise
 * summary from the quoted candidate lines, or full when no locatable
 * summary can be built. Read probes never reach this policy. The reason
 * carries the probabilities for audit; no code reads them back.
 *
 * Intentionally not in the Log: the thresholds. The tier and the reason
 * are the record.
 */

import {
  clampSummary,
  isLocatableSummary,
} from "../../compiler/score-common.js";
import type { ScoreDecision } from "../types.js";
import type { TriageJudgment } from "./questions.js";

/**
 * Noul cut-offs for the tier policy, each a probability in [0, 1].
 * `full`: a failure report or new location at or above it keeps the text.
 * `low`: the failure, location, and every keep answer must be below it
 * for none. `keep`: a candidate line at or above it may be quoted in a
 * summary.
 */
export type JevThresholds = {
  full: number;
  low: number;
  keep: number;
};

/**
 * Default cut-offs. Dropping evidence the agent later needs costs a
 * re-read or a wrong step, while keeping text costs only tokens, so the
 * bar to keep is low and the bar to drop is high: `full` sits below an
 * even call, `low` needs every judgment to be a clear no, and `keep`
 * needs a line to be more likely useful than not.
 */
export const DEFAULT_JEV_THRESHOLDS: Readonly<JevThresholds> = {
  full: 0.4,
  low: 0.2,
  keep: 0.5,
};

/**
 * The decision for a chunk whose result the tool already marked
 * `isError`: full, reason `scorer:jev:full:is_error`. No request is sent;
 * a failure is evidence the next step needs.
 */
export function errorDecision(id: string): ScoreDecision {
  return { id, tier: "full", reason: "scorer:jev:full:is_error" };
}

/** Most candidate lines one summary quotes. */
const SUMMARY_LINES = 3;
/** Joins quoted lines in a summary note. */
const SUMMARY_SEPARATOR = "; ";

/**
 * The decision for chunk `id`. A summary quotes up to `SUMMARY_LINES`
 * candidates with the highest `keep` at or above the threshold, in output
 * order, joined with `; ` and cut to the summary limit; a note that fails
 * `isLocatableSummary` turns the tier into full. The reason is
 * `scorer:jev:<tier>:failure=<p>,locate=<p>` plus `,keep=<max p>` when
 * the chunk had candidates, each rounded to two decimals.
 */
export function decideTier(
  id: string,
  judgment: TriageJudgment,
  candidates: readonly string[],
  thresholds: JevThresholds,
): ScoreDecision {
  const reasonFor = (tier: string) => `scorer:jev:${tier}:${audit(judgment)}`;
  if (
    judgment.failure >= thresholds.full ||
    judgment.locate >= thresholds.full
  ) {
    return { id, tier: "full", reason: reasonFor("full") };
  }
  if (isEveryAnswerLow(judgment, thresholds.low)) {
    return { id, tier: "none", reason: reasonFor("none") };
  }
  const note = summaryNote(judgment.keep, candidates, thresholds.keep);
  if (note === undefined) {
    return { id, tier: "full", reason: reasonFor("full") };
  }
  return { id, tier: "summary", summary: note, reason: reasonFor("summary") };
}

function isEveryAnswerLow(judgment: TriageJudgment, low: number): boolean {
  return (
    judgment.failure < low &&
    judgment.locate < low &&
    judgment.keep.every((p) => p < low)
  );
}

/** The quoted-line note, or undefined when none qualifies or it is not locatable. */
function summaryNote(
  keep: readonly number[],
  candidates: readonly string[],
  threshold: number,
): string | undefined {
  const chosen = keep
    .map((p, index) => ({ p, index }))
    .filter((item) => item.p >= threshold && item.index < candidates.length)
    .sort((a, b) => b.p - a.p || a.index - b.index)
    .slice(0, SUMMARY_LINES)
    .sort((a, b) => a.index - b.index);
  if (chosen.length === 0) return undefined;
  const note = clampSummary(
    chosen.map((item) => candidates[item.index]).join(SUMMARY_SEPARATOR),
  );
  return isLocatableSummary(note) ? note : undefined;
}

function audit(judgment: TriageJudgment): string {
  const parts = [
    `failure=${judgment.failure.toFixed(2)}`,
    `locate=${judgment.locate.toFixed(2)}`,
  ];
  if (judgment.keep.length > 0) {
    parts.push(`keep=${Math.max(...judgment.keep).toFixed(2)}`);
  }
  return parts.join(",");
}
