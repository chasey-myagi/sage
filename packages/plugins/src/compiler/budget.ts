/**
 * Pure budget allocation over claims.
 *
 * Contract: pinned claims take their highest tier (`full` when present) and
 * never move. Everyone else starts at their highest available tier. While the
 * total exceeds the budget, the lowest-priority claim that currently sits on
 * the highest tier drops one available tier. Missing tiers are skipped. `none`
 * is the floor and contributes no tokens. `overrun` is how far `used` still
 * sits above the budget after that loop stops.
 *
 * Intentionally not in the Log: this function. Callers copy `records` onto the
 * manifest entry.
 */

import type { ClaimRecord } from "@sage/core";
import type { Claim, Tier } from "./claim.js";

/** Estimates the tokens of a text. Must be deterministic. */
export type TokenCounter = (text: string) => number;

const LADDER: readonly Tier[] = ["full", "summary", "line", "none"];

/** Default estimator: one token per 4 characters, rounded up. */
export function defaultTokenCount(text: string): number {
  return Math.ceil(text.length / 4);
}

/**
 * Chooses a tier for every claim so the total fits `budget` when it can.
 * A duplicate id keeps the last claim. `records` are in priority order
 * with reason `pinned`, `fits`, or `budget:downgraded`; `chosen` maps id to
 * tier.
 */
export function allocate(
  claims: readonly Claim[],
  budget: number,
  count: TokenCounter,
): { records: ClaimRecord[]; chosen: Map<string, Tier>; overrun: number } {
  const byId = new Map<string, Claim>();
  for (const claim of claims) byId.set(claim.id, claim);
  const ordered = [...byId.values()].sort(
    (left, right) =>
      right.priority - left.priority || left.id.localeCompare(right.id),
  );

  const chosen = new Map<string, Tier>();
  for (const claim of ordered) chosen.set(claim.id, highest(claim));

  const usedOf = () => {
    let total = 0;
    for (const claim of ordered) {
      const tier = chosen.get(claim.id) ?? "none";
      total += tierTokens(claim, tier, count);
    }
    return total;
  };

  let used = usedOf();
  const steps = ordered.length * LADDER.length + 1;
  for (let step = 0; step < steps && used > budget; step += 1) {
    const open = ordered.filter(
      (claim) => !claim.pinned && chosen.get(claim.id) !== "none",
    );
    if (open.length === 0) break;
    let top = LADDER.length;
    for (const claim of open) {
      const rank = LADDER.indexOf(chosen.get(claim.id) ?? "none");
      if (rank < top) top = rank;
    }
    const victims = open
      .filter((claim) => LADDER.indexOf(chosen.get(claim.id) ?? "none") === top)
      .sort(
        (left, right) =>
          left.priority - right.priority || left.id.localeCompare(right.id),
      );
    const victim = victims[0];
    if (!victim) break;
    const current = chosen.get(victim.id) ?? "none";
    const next = stepDown(victim, current);
    if (next === current) break;
    chosen.set(victim.id, next);
    used = usedOf();
  }

  const records: ClaimRecord[] = ordered.map((claim) => {
    const tier = chosen.get(claim.id) ?? "none";
    const started = highest(claim);
    return {
      id: claim.id,
      tier,
      reason: claim.pinned
        ? "pinned"
        : tier === started
          ? "fits"
          : "budget:downgraded",
      tokens: tierTokens(claim, tier, count),
    };
  });

  return { records, chosen, overrun: Math.max(0, used - budget) };
}

function highest(claim: Claim): Tier {
  for (const tier of LADDER) {
    if (tier === "none") return "none";
    if (claim.tiers[tier] !== undefined) return tier;
  }
  return "none";
}

function stepDown(claim: Claim, current: Tier): Tier {
  const start = LADDER.indexOf(current);
  for (let index = start + 1; index < LADDER.length; index += 1) {
    const tier = LADDER[index] ?? "none";
    if (tier === "none") return "none";
    if (claim.tiers[tier] !== undefined) return tier;
  }
  return "none";
}

function tierTokens(claim: Claim, tier: Tier, count: TokenCounter): number {
  if (tier === "none") return 0;
  const text = claim.tiers[tier];
  if (text === undefined) return 0;
  return count(text);
}
