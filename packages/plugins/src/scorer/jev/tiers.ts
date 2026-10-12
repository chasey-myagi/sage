/**
 * Turns one chunk's Noul answers into a tier. Jev judges; this code owns
 * the policy.
 *
 * Guarantees: full when a failure report or an edit target reaches
 * `full`; none when the output is a broad listing at `none` or above and
 * every other fixed judgment is below `low`; otherwise summary from the
 * quoted candidate lines, or full when no locatable summary can be built.
 * The reason carries the probabilities for audit; no code reads them back.
 *
 * Intentionally not in the Log: the thresholds. The tier and the reason
 * are the record.
 */

import {
  clampSummary,
  isLocatableSummary,
} from "../../compiler/score-common.js";
import type { ScoreDecision } from "../types.js";
import type { Judgment } from "./questions.js";

/**
 * Noul cut-offs for the tier policy, each a probability in [0, 1].
 * `full`: a failure report or edit target at or above it keeps the text.
 * `none`: a broad listing at or above it may be dropped. `low`: every
 * other fixed judgment must be below it for none. `keep`: a candidate
 * line at or above it may be quoted in a summary.
 */
export type JevThresholds = {
  full: number;
  none: number;
  low: number;
  keep: number;
};

/**
 * Default cut-offs. Dropping evidence the agent later needs costs a
 * re-read or a wrong step, while keeping text costs only tokens, so the
 * bar to keep is low and the bar to drop is high: `full` sits below an
 * even call, `none` needs a confident listing, `low` needs the other
 * judgments to be clear noes, and `keep` needs a line to be more likely
 * useful than not.
 */
export const DEFAULT_JEV_THRESHOLDS: Readonly<JevThresholds> = {
  full: 0.4,
  none: 0.8,
  low: 0.2,
  keep: 0.5,
};

/** Most candidate lines one summary quotes. */
const SUMMARY_LINES = 3;
/** Joins quoted lines in a summary note. */
const SUMMARY_SEPARATOR = "; ";

/**
 * The decision for chunk `id`. A summary quotes up to `SUMMARY_LINES`
 * candidates with the highest `keep` at or above the threshold, in output
 * order, joined with `; ` and cut to the summary limit; a note that fails
 * `isLocatableSummary` turns the tier into full. The reason is
 * `scorer:jev:<tier>:failure=<p>,edit=<p>,lookup=<p>,broad=<p>` plus
 * `,keep=<max p>` when the chunk had candidates, each rounded to two
 * decimals.
 */
export function decideTier(
  id: string,
  judgment: Judgment,
  candidates: readonly string[],
  thresholds: JevThresholds,
): ScoreDecision {
  const reasonFor = (tier: string) => `scorer:jev:${tier}:${audit(judgment)}`;
  if (judgment.failure >= thresholds.full || judgment.edit >= thresholds.full) {
    return { id, tier: "full", reason: reasonFor("full") };
  }
  if (isDroppableListing(judgment, thresholds)) {
    return { id, tier: "none", reason: reasonFor("none") };
  }
  const note = summaryNote(judgment.keep, candidates, thresholds.keep);
  if (note === undefined) {
    return { id, tier: "full", reason: reasonFor("full") };
  }
  return { id, tier: "summary", summary: note, reason: reasonFor("summary") };
}

function isDroppableListing(
  judgment: Judgment,
  thresholds: JevThresholds,
): boolean {
  return (
    judgment.broad >= thresholds.none &&
    judgment.failure < thresholds.low &&
    judgment.edit < thresholds.low &&
    judgment.lookup < thresholds.low
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

function audit(judgment: Judgment): string {
  const parts = [
    `failure=${judgment.failure.toFixed(2)}`,
    `edit=${judgment.edit.toFixed(2)}`,
    `lookup=${judgment.lookup.toFixed(2)}`,
    `broad=${judgment.broad.toFixed(2)}`,
  ];
  if (judgment.keep.length > 0) {
    parts.push(`keep=${Math.max(...judgment.keep).toFixed(2)}`);
  }
  return parts.join(",");
}
