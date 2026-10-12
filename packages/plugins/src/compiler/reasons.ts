/**
 * Chunk reason strings: how the compiler composes them and the one decoder
 * that reads them back.
 *
 * Contract: a chunk reason is a root such as `recent`, `kept`,
 * `superseded_by:<id>`, `retry_intermediate`, `budget:oldest-first`,
 * `scorer:entry:<tier>`, or a scorer's own text, followed by zero or more
 * `;<suffix>` parts. The manifest persists only this string, so a decision
 * read back from the Log carries no other marker of the rule that made it.
 * `reasonKind` is the only place that decodes a reason; code branches on
 * its result, never on the raw string. The composers here keep every
 * reason byte-identical to what earlier manifests recorded.
 *
 * Intentionally not in the Log: `ReasonKind`. The string is the record.
 */

import type { ScoreDecision } from "../scorer/types.js";
import type { Tier } from "./claim.js";
import type { Rates } from "./decide.js";

/** The rule a chunk reason's root names. */
type ReasonKind =
  | "recent"
  | "kept"
  | "superseded"
  | "retry"
  | "budget"
  | "entry-scorer"
  | "other";

/** Decodes the root of a chunk reason into the rule that produced it. */
export function reasonKind(reason: string): ReasonKind {
  const root = reason.split(";")[0] ?? reason;
  if (root === "recent") return "recent";
  if (root === "kept") return "kept";
  if (root === "retry_intermediate") return "retry";
  if (root === "budget:oldest-first") return "budget";
  if (root.startsWith("superseded_by:")) return "superseded";
  if (root.startsWith("scorer:entry:")) return "entry-scorer";
  return "other";
}

/**
 * Appends each stamp that `reason` does not already contain as a
 * `;`-separated part. The tier is not affected.
 */
export function withStamps(reason: string, stamps: readonly string[]): string {
  const extra = stamps.filter((stamp) => !reason.split(";").includes(stamp));
  if (extra.length === 0) return reason;
  return `${reason};${extra.join(";")}`;
}

/**
 * Reason for an entry-scored chunk: `scorer:entry:<tier>`, then the
 * scorer's own non-empty `scorerReason`, then the `expanded_at:` stamps of
 * the previous reason; other suffixes are dropped. A scorer reason must
 * not contain `;`.
 */
export function entryReason(
  reason: string,
  tier: Tier,
  scorerReason?: string,
): string {
  const stamps = reason
    .split(";")
    .slice(1)
    .filter((part) => part.startsWith("expanded_at:"));
  const parts = [`scorer:entry:${tier}`];
  if (scorerReason !== undefined && scorerReason.length > 0) {
    parts.push(scorerReason);
  }
  return [...parts, ...stamps].join(";");
}

/**
 * Reason for an entry-scored summary whose note failed the locatability
 * check and fell back to full.
 */
export function summaryRejectedReason(
  reason: string,
  scorerReason?: string,
): string {
  return `${entryReason(reason, "full", scorerReason)};summary-rejected`;
}

/**
 * Reason for a rebuild-scored chunk: the scorer's non-empty `reason`, else
 * `scorer:model`, plus every non-empty suffix of the previous reason.
 */
export function modelReason(reason: string, decision: ScoreDecision): string {
  const root =
    decision.reason !== undefined && decision.reason.length > 0
      ? decision.reason
      : "scorer:model";
  const extra = reason
    .split(";")
    .slice(1)
    .filter((part) => part.length > 0);
  if (extra.length === 0) return root;
  return `${root};${extra.join(";")}`;
}

/**
 * Appends `;pricing:fallback` to an epoch decision reason when the rates
 * came from the fallback table rather than the model catalog.
 */
export function tagPricing(reason: string, rates: Rates): string {
  if (rates.source !== "fallback") return reason;
  return `${reason};pricing:fallback`;
}
