/**
 * Pure cost check for opening a new view epoch.
 *
 * Contract: rates are relative to the uncached input price. `ratesFor`
 * reads them from the call's pricing (the object this turn's
 * request_header records) and marks each rate's source; a missing price
 * uses the conservative defaults below. The rebuild cost is
 *   extraCost = new * write - old * read + postRebuildCost
 * where postRebuildCost = expandTokens * write prices the omitted tokens a
 * rebuild is expected to fetch back with `expand`; expandTokens never
 * exceeds the drop (old - new). savingsPerTurn =
 * (old - new) * read. paybackTurns is ceil(extraCost / savingsPerTurn),
 * null when savings are not positive, 0 when extraCost is not positive.
 * Rules, in order: overContext rebuilds (`context-window-guard`); a drop
 * below minSavingsRatio * old does not (`savings-too-small`); a rebuild
 * the rules below would approve waits while the epoch is younger than
 * minRebuildInterval turns (`min-interval:`); extraCost <= 0 rebuilds
 * (`immediate`); otherwise rebuild when paybackTurns <= remainingTurns.
 * This module estimates none of its inputs.
 *
 * Intentionally not in the Log: these functions. The compiler copies the
 * decision onto the manifest.
 */

import { assertNever, type Pricing } from "@sage/provider/contract";

/** Where a cache rate came from: the call's pricing, or a default here. */
export type RateSource = "pricing" | "default";

/** Cache price multipliers relative to the uncached input price. */
export type Rates = {
  /** Cache-read price as a fraction of the input price. */
  read: number;
  /** Cache-write price as a multiple of the input price; never below 1. */
  write: number;
  readSource: RateSource;
  writeSource: RateSource;
};

/**
 * Expected expand traffic after a rebuild. `observed` divides this
 * session's successful expands by the epochs so far and averages their
 * result tokens. `prior`, used before any expand is observed, assumes
 * `share` of the tokens a rebuild drops come back.
 */
export type ExpandEstimate =
  | {
      source: "observed";
      expandsPerEpoch: number;
      /** Mean estimated tokens of one expand result. */
      tokensPerExpand: number;
    }
  | { source: "prior"; share: number };

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
  readRateSource: RateSource;
  writeRateSource: RateSource;
  savingsPerTurn: number;
  /** Includes `postRebuildCost`. Input-price-equivalent tokens. */
  extraCost: number;
  /**
   * Expected cost of the expands a rebuild invites, in
   * input-price-equivalent tokens: expected expand tokens, capped at
   * `oldTokens - newTokens`, times `writeRate`.
   */
  postRebuildCost: number;
  expandEstimate: ExpandEstimate;
  /** Turns since the current epoch's first manifest; 0 on that turn. */
  turnsSinceEpochOpen: number;
  /** Fewest turns an epoch lasts before a cost rebuild may end it. */
  minRebuildInterval: number;
  /** Turns until a rebuild pays for itself; null without savings. */
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

/**
 * Read rate when the call has no pricing: the common cache-read discount.
 * A lower read rate only makes rebuilds rarer.
 */
export const DEFAULT_READ_RATE = 0.1;

/**
 * Write rate when the pricing lists no cache-write price (no pricing, or
 * `cacheWritePerMTok` 0): the highest write multiplier in the built-in
 * catalog. A zero price cannot tell "no surcharge" from "not published",
 * and such endpoints report no cache creation in usage either, so nothing
 * confirms a cheaper rate. A higher write rate only makes rebuilds rarer.
 */
export const DEFAULT_WRITE_RATE = 1.25;

/**
 * Share of the tokens a rebuild drops that is assumed to come back through
 * `expand` before any expand is observed. A share keeps the prior in
 * proportion to the drop at every view size.
 */
export const PRIOR_EXPAND_SHARE = 0.1;

/**
 * Derives cache rates from the call's pricing. Without pricing, or with a
 * zero input price, both rates are defaults. A zero cache-write price keeps
 * the pricing's read rate and uses `DEFAULT_WRITE_RATE`. A listed write
 * price below the input price is raised to it: the first fill still pays
 * the miss price.
 */
export function ratesFor(pricing: Pricing | null): Rates {
  if (!pricing || pricing.inputPerMTok <= 0) {
    return {
      read: DEFAULT_READ_RATE,
      write: DEFAULT_WRITE_RATE,
      readSource: "default",
      writeSource: "default",
    };
  }
  const input = pricing.inputPerMTok;
  const read = pricing.cacheReadPerMTok / input;
  if (pricing.cacheWritePerMTok <= 0) {
    return {
      read,
      write: DEFAULT_WRITE_RATE,
      readSource: "pricing",
      writeSource: "default",
    };
  }
  return {
    read,
    write: Math.max(pricing.cacheWritePerMTok, input) / input,
    readSource: "pricing",
    writeSource: "pricing",
  };
}

/**
 * Expand estimate from the result tokens of this session's successful
 * expands and the number of epochs so far, floored at 1. No observations
 * yield the prior.
 */
export function expandEstimateFor(
  resultTokens: readonly number[],
  epochs: number,
): ExpandEstimate {
  if (resultTokens.length === 0) {
    return { source: "prior", share: PRIOR_EXPAND_SHARE };
  }
  const total = resultTokens.reduce((sum, tokens) => sum + tokens, 0);
  return {
    source: "observed",
    expandsPerEpoch: resultTokens.length / Math.max(1, epochs),
    tokensPerExpand: total / resultTokens.length,
  };
}

/** Input to the epoch cost check. Token counts are projected input tokens. */
type EpochInput = {
  oldTokens: number;
  newTokens: number;
  remainingTurns: number;
  rates: Rates;
  minSavingsRatio: number;
  contextWindow: number;
  overContext: boolean;
  expands: ExpandEstimate;
  turnsSinceEpochOpen: number;
  minRebuildInterval: number;
};

/**
 * Runs the cost check and returns only the decision.
 * Throws when `contextWindow` is not finite.
 */
export function decideEpoch(input: EpochInput): EpochDecision {
  return classifyEpoch(input).decision;
}

/**
 * Runs the cost check and also reports which rule decided. A rebuild held
 * by the minimum interval reports `no-rebuild`.
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
  const costed = costRule(decision, input.remainingTurns);
  if (
    costed.decision.rebuild &&
    input.turnsSinceEpochOpen < input.minRebuildInterval
  ) {
    return {
      decision: {
        ...costed.decision,
        rebuild: false,
        reason: `min-interval:${input.turnsSinceEpochOpen}<${input.minRebuildInterval}`,
      },
      trigger: "no-rebuild",
    };
  }
  return costed;
}

/** The `immediate` and `payback` rules on a costed decision. */
function costRule(
  decision: EpochDecision,
  remainingTurns: number,
): { decision: EpochDecision; trigger: EpochTrigger } {
  if (decision.extraCost <= 0) {
    return {
      decision: { ...decision, rebuild: true, reason: "immediate" },
      trigger: "immediate",
    };
  }
  const { paybackTurns } = decision;
  if (paybackTurns === null) return { decision, trigger: "no-rebuild" };
  const rebuild = paybackTurns <= remainingTurns;
  return {
    decision: {
      ...decision,
      rebuild,
      reason: rebuild
        ? `payback:${paybackTurns}<=${remainingTurns}`
        : `payback:${paybackTurns}>${remainingTurns}`,
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
  const { read, write } = input.rates;
  const drop = Math.max(0, input.oldTokens - input.newTokens);
  const postRebuildCost = expandTokens(input.expands, drop) * write;
  const savingsPerTurn = (input.oldTokens - input.newTokens) * read;
  const extraCost =
    input.newTokens * write - input.oldTokens * read + postRebuildCost;
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
    readRateSource: input.rates.readSource,
    writeRateSource: input.rates.writeSource,
    savingsPerTurn,
    extraCost,
    postRebuildCost,
    expandEstimate: { ...input.expands },
    turnsSinceEpochOpen: input.turnsSinceEpochOpen,
    minRebuildInterval: input.minRebuildInterval,
    paybackTurns,
    reason: "savings-too-small",
  };
}

/** Tokens a rebuild dropping `drop` tokens is expected to fetch back. */
function expandTokens(estimate: ExpandEstimate, drop: number): number {
  switch (estimate.source) {
    case "prior":
      return estimate.share * drop;
    case "observed":
      return Math.min(
        estimate.expandsPerEpoch * estimate.tokensPerExpand,
        drop,
      );
    default:
      return assertNever(estimate);
  }
}
