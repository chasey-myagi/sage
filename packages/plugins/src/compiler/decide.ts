/**
 * Pure cost check for opening a new view epoch.
 *
 * Contract: read = cacheRead/input. write = max(cacheWrite, input) / input.
 * A zero cache-write surcharge is not free: the first fill still pays the
 * miss price. Null pricing falls back to 0.1 and 1.25.
 * savingsPerTurn = (old - new) * read.
 * extraCost = new * write - old * read. paybackTurns is
 * ceil(extraCost / savingsPerTurn), or null when savings are not positive,
 * or 0 when extraCost is not positive. overContext rebuilds with reason
 * `context-window-guard`. A drop smaller than minSavingsRatio * oldTokens
 * does not rebuild (`savings-too-small`). extraCost <= 0 rebuilds with
 * reason `immediate`. Otherwise rebuild when paybackTurns <= remainingTurns.
 * `remainingTurns` is an input. This function does not estimate it.
 *
 * Intentionally not in the Log: this function. The compiler copies the
 * decision onto the manifest.
 */

import type { Pricing } from "@sage/provider/contract";

/** Cache price multipliers relative to the uncached input price. */
export type Rates = {
  /** Cache-read price as a fraction of the input price. */
  read: number;
  /** Cache-write price as a multiple of the input price; never below 1. */
  write: number;
  /** `fallback` when the model has no catalog pricing. */
  source: "catalog" | "fallback";
};

/**
 * The cost check for one turn. An epoch is the span of turns during which
 * the view prefix stays byte-identical so the provider's prefix cache keeps
 * hitting; a rebuild recompiles the prefix and starts the next epoch.
 * The compiler copies this object onto the manifest as `epochDecision`.
 */
export type EpochDecision = {
  rebuild: boolean;
  /** Projected input tokens of the frozen view. */
  oldTokens: number;
  /** Projected input tokens of a from-scratch recompile. */
  newTokens: number;
  remainingTurns: number;
  readRate: number;
  writeRate: number;
  savingsPerTurn: number;
  extraCost: number;
  /** Turns until a rebuild pays for its cache write; null without savings. */
  paybackTurns: number | null;
  /** Human-readable rule that decided; Log text, not a branch key. */
  reason: string;
};

/**
 * Which rule in `classifyEpoch` produced the decision. Callers branch on
 * this instead of parsing `EpochDecision.reason`.
 */
export type EpochTrigger =
  | "context-window-guard"
  | "immediate"
  | "payback"
  | "no-rebuild";

/**
 * Reason text of a guard rebuild. It is also the `view_epoch` open reason
 * of an epoch that the guard opened.
 */
export const CONTEXT_WINDOW_GUARD_REASON = "context-window-guard";

const FALLBACK: Rates = { read: 0.1, write: 1.25, source: "fallback" };

/** Derives cache rates from catalog pricing, or the fallback 0.1 / 1.25. */
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

/** Input to the epoch cost check. Token counts are projected input tokens. */
export type EpochInput = {
  oldTokens: number;
  newTokens: number;
  remainingTurns: number;
  rates: Rates;
  minSavingsRatio: number;
  contextWindow: number;
  overContext: boolean;
};

/**
 * Runs the cost check and returns only the decision.
 * Throws when `contextWindow` is not finite.
 */
export function decideEpoch(input: EpochInput): EpochDecision {
  return classifyEpoch(input).decision;
}

/**
 * Runs the cost check and also reports which rule decided.
 * Throws when `contextWindow` is not finite.
 */
export function classifyEpoch(input: EpochInput): {
  decision: EpochDecision;
  trigger: EpochTrigger;
} {
  const decision = epochCosts(input);
  if (input.overContext) {
    return {
      decision: {
        ...decision,
        rebuild: true,
        reason: CONTEXT_WINDOW_GUARD_REASON,
      },
      trigger: "context-window-guard",
    };
  }
  if (
    input.oldTokens - input.newTokens <
    input.minSavingsRatio * input.oldTokens
  ) {
    return { decision, trigger: "no-rebuild" };
  }
  if (decision.extraCost <= 0) {
    return {
      decision: { ...decision, rebuild: true, reason: "immediate" },
      trigger: "immediate",
    };
  }
  const { paybackTurns } = decision;
  if (paybackTurns === null) return { decision, trigger: "no-rebuild" };
  const rebuild = paybackTurns <= input.remainingTurns;
  return {
    decision: {
      ...decision,
      rebuild,
      reason: rebuild
        ? `payback:${paybackTurns}<=${input.remainingTurns}`
        : `payback:${paybackTurns}>${input.remainingTurns}`,
    },
    trigger: rebuild ? "payback" : "no-rebuild",
  };
}

/**
 * Computes the cost fields with `rebuild: false` and reason
 * `savings-too-small`. Throws when `contextWindow` is not finite.
 */
function epochCosts(input: EpochInput): EpochDecision {
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
  return {
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
}
