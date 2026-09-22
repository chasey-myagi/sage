/**
 * Pure cost check for opening a new view epoch.
 *
 * Contract: read = cacheRead/input. write = max(cacheWrite, input) / input.
 * A zero cache-write surcharge is not free: the first fill still pays the
 * miss price (无附加费 ≠ 免费，首次仍付未命中价). Null pricing falls back
 * to 0.1 and 1.25. savingsPerTurn = (old - new) * read.
 * extraCost = new * write - old * read. paybackTurns is
 * ceil(extraCost / savingsPerTurn), or null when savings are not positive,
 * or 0 when extraCost is not positive. overContext rebuilds with reason
 * `context-window-guard`. A drop smaller than minSavingsRatio * oldTokens
 * does not rebuild (`savings-too-small`). extraCost <= 0 rebuilds with
 * reason `immediate`. Otherwise rebuild when paybackTurns <= remainingTurns.
 *
 * Intentionally not in the Log: this function. The compiler copies the
 * decision onto the manifest.
 */

import type { Pricing } from "@sage/provider/contract";

export type Rates = {
  read: number;
  write: number;
  source: "catalog" | "fallback";
};

export type EpochDecision = {
  rebuild: boolean;
  oldTokens: number;
  newTokens: number;
  remainingTurns: number;
  readRate: number;
  writeRate: number;
  savingsPerTurn: number;
  extraCost: number;
  paybackTurns: number | null;
  reason: string;
};

const FALLBACK: Rates = { read: 0.1, write: 1.25, source: "fallback" };

export function ratesFor(pricing: Pricing | null): Rates {
  if (!pricing || pricing.inputPerMTok === 0) return { ...FALLBACK };
  return {
    read: pricing.cacheReadPerMTok / pricing.inputPerMTok,
    write:
      Math.max(pricing.cacheWritePerMTok, pricing.inputPerMTok) /
      pricing.inputPerMTok,
    source: "catalog",
  };
}

export function decideEpoch(input: {
  oldTokens: number;
  newTokens: number;
  remainingTurns: number;
  rates: Rates;
  minSavingsRatio: number;
  contextWindow: number;
  overContext: boolean;
}): EpochDecision {
  if (!Number.isFinite(input.contextWindow)) {
    throw new Error("contextWindow must be finite");
  }
  const read = input.rates.read;
  const write = input.rates.write;
  const savingsPerTurn = (input.oldTokens - input.newTokens) * read;
  const extraCost = input.newTokens * write - input.oldTokens * read;
  let paybackTurns: number | null;
  if (savingsPerTurn <= 0) paybackTurns = null;
  else if (extraCost <= 0) paybackTurns = 0;
  else paybackTurns = Math.ceil(extraCost / savingsPerTurn);

  const decision: EpochDecision = {
    rebuild: false,
    oldTokens: input.oldTokens,
    newTokens: input.newTokens,
    remainingTurns: input.remainingTurns,
    readRate: read,
    writeRate: write,
    savingsPerTurn,
    extraCost,
    paybackTurns,
    reason: "savings-too-small",
  };

  if (input.overContext) {
    return { ...decision, rebuild: true, reason: "context-window-guard" };
  }
  if (
    input.oldTokens - input.newTokens <
    input.minSavingsRatio * input.oldTokens
  ) {
    return decision;
  }
  if (extraCost <= 0) {
    return { ...decision, rebuild: true, reason: "immediate" };
  }
  if (paybackTurns !== null && paybackTurns <= input.remainingTurns) {
    return {
      ...decision,
      rebuild: true,
      reason: `payback:${paybackTurns}<=${input.remainingTurns}`,
    };
  }
  if (paybackTurns !== null) {
    return {
      ...decision,
      rebuild: false,
      reason: `payback:${paybackTurns}>${input.remainingTurns}`,
    };
  }
  return decision;
}
