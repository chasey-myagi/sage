/**
 * Epoch choice for one turn: project both candidate views onto measured
 * tokens, run the cost check, and apply the compiler's own overrides.
 *
 * Contract: an epoch keeps the view prefix byte-identical so the provider's
 * prefix cache keeps hitting. The only rebuild allowed on an epoch's first
 * turn is the context-window guard. A payback rebuild also waits until the
 * epoch has lasted the payback it was opened on, which is read from the
 * epoch's first manifest; a guard-opened epoch commits to none. The guard
 * fires when the projected
 * frozen view exceeds `CONTEXT_WINDOW_GUARD_RATIO` of the context window,
 * and does not fire again inside an epoch the guard itself opened.
 *
 * Intentionally not in the Log: the projection function, the trigger, and
 * the route policy.
 * The decision is copied onto the manifest as `epochDecision`, and the
 * open reason onto `view_epoch`.
 */

import type { EpochStart } from "@sage/core";
import { listModels } from "@sage/provider";
import type { ModelRef } from "@sage/provider/contract";
import type { RoutePolicy } from "../routing/types.js";
import type { Calibration } from "./calibrate.js";
import type { Chunk } from "./chunks.js";
import {
  CONTEXT_WINDOW_GUARD_REASON,
  classifyEpoch,
  type EpochDecision,
  type EpochTrigger,
  type ExpandEstimate,
  type Rates,
} from "./decide.js";
import { tagPricing } from "./reasons.js";
import type { ChunkRow } from "./rows.js";

/**
 * Fraction of the model's context window a projected view may fill before
 * the context-window guard forces a rebuild.
 */
const CONTEXT_WINDOW_GUARD_RATIO = 0.9;
/** Default `expectedTurns`: assumed length of a task in turns. */
export const DEFAULT_EXPECTED_TURNS = 20;
/**
 * Default `alpha`: a long task assumes `alpha` times its elapsed turns
 * still remain.
 */
export const DEFAULT_ALPHA = 0.5;
/**
 * Default `minSavingsRatio`: smallest token drop, as a fraction of the
 * frozen view, worth a rebuild.
 */
export const DEFAULT_MIN_SAVINGS_RATIO = 0.2;

/**
 * Maps estimated tokens to measured input tokens:
 * `overheadTokens + used × ratio`. `overheadTokens` is the fixed overhead
 * from the epoch's first measured input, floored at 0 so a negative
 * overhead cannot make the new view's tokens negative and reopen as
 * immediate. `ratio` is `(measured − overheadTokens) / budget.used`,
 * applied even when above 1. Without a calibration, overhead 0 and ratio 1.
 */
export type Projection = (used: number) => number;

/** Builds the projection for this turn's calibration. */
export function projectionFor(
  calibration: Calibration | undefined,
): Projection {
  const overhead = calibration?.overheadTokens ?? 0;
  const ratio = calibration?.ratio ?? 1;
  return (used) => overhead + used * ratio;
}

/**
 * Turns the cost check may count on: turns already started are
 * max(0, turn − 1), and the remainder is max(1, expectedTurns − started,
 * ceil(alpha × started)), so a long task assumes about `alpha` times as
 * many turns again.
 */
export function remainingTurnsFor(
  turn: number,
  expectedTurns: number,
  alpha: number,
): number {
  const started = Math.max(0, turn - 1);
  return Math.max(1, expectedTurns - started, Math.ceil(alpha * started));
}

/** The epoch decision plus the structured facts later stages branch on. */
export type EpochChoice = {
  decision: EpochDecision;
  /** Which rule decided; `no-rebuild` after the first-turn override. */
  trigger: EpochTrigger;
  /** The current epoch was opened by the context-window guard. */
  isGuardOpenedEpoch: boolean;
  /** The projected frozen view exceeds the guard threshold. */
  isFrozenOverWindow: boolean;
};

/**
 * Runs the cost check on the frozen view (`oldTokens`) against a
 * from-scratch recompile (`newTokens`), both projected. Without pricing
 * the reason gets `;pricing:fallback`. Without a freeze
 * (`hasFrozen` false) any rebuild other than the guard becomes
 * `savings-too-small`.
 */
export function chooseEpoch(input: {
  frozenUsed: number;
  recompiledUsed: number;
  hasFrozen: boolean;
  project: Projection;
  rates: Rates;
  remainingTurns: number;
  minSavingsRatio: number;
  contextWindow: number;
  epochOpenReason: string | undefined;
  expands: ExpandEstimate;
  turnsSinceEpochOpen: number;
  /** From the epoch's first manifest; ignored in a guard-opened epoch. */
  committedPaybackTurns: number;
}): EpochChoice {
  const isGuardOpenedEpoch =
    input.epochOpenReason === CONTEXT_WINDOW_GUARD_REASON;
  const isFrozenOverWindow = exceedsGuard(
    input.project(input.frozenUsed),
    input.contextWindow,
  );
  const classified = classifyEpoch({
    oldTokens: input.project(input.frozenUsed),
    newTokens: input.project(input.recompiledUsed),
    remainingTurns: input.remainingTurns,
    rates: input.rates,
    minSavingsRatio: input.minSavingsRatio,
    contextWindow: input.contextWindow,
    overContext: isFrozenOverWindow && !isGuardOpenedEpoch,
    expands: input.expands,
    turnsSinceEpochOpen: input.turnsSinceEpochOpen,
    committedPaybackTurns: isGuardOpenedEpoch ? 0 : input.committedPaybackTurns,
  });
  const decision: EpochDecision = {
    ...classified.decision,
    reason: tagPricing(classified.decision.reason, input.rates),
  };
  const choice = {
    trigger: classified.trigger,
    isGuardOpenedEpoch,
    isFrozenOverWindow,
  };
  if (
    !input.hasFrozen &&
    classified.trigger !== "context-window-guard" &&
    decision.rebuild
  ) {
    return {
      ...choice,
      trigger: "no-rebuild",
      decision: {
        ...decision,
        rebuild: false,
        reason: tagPricing("savings-too-small", input.rates),
      },
    };
  }
  return { ...choice, decision };
}

/**
 * `view_epoch` reason for an epoch this turn opens: `context-window-guard`,
 * `cost:immediate`, or `cost:<paybackTurns>`.
 */
function openReason(choice: EpochChoice): string {
  if (choice.trigger === "context-window-guard") {
    return CONTEXT_WINDOW_GUARD_REASON;
  }
  if (choice.trigger === "immediate") return "cost:immediate";
  if (choice.decision.paybackTurns === null) return "cost:immediate";
  return `cost:${choice.decision.paybackTurns}`;
}

/**
 * `budget.overrun` for the chosen view: how far `used` exceeds the compile
 * budget. When that is 0 but a guard-related view (a guard rebuild this
 * turn, or a frozen view over the threshold inside a guard-opened epoch)
 * is still projected over the threshold, the excess over the threshold is
 * recorded instead.
 */
export function viewOverrun(input: {
  used: number;
  budget: number;
  project: Projection;
  contextWindow: number;
  choice: EpochChoice;
}): number {
  const overrun = Math.max(0, input.used - input.budget);
  const projected = input.project(input.used);
  const { choice } = input;
  const isGuardView =
    choice.trigger === "context-window-guard" ||
    (choice.isGuardOpenedEpoch && choice.isFrozenOverWindow);
  if (
    overrun === 0 &&
    exceedsGuard(projected, input.contextWindow) &&
    isGuardView
  ) {
    return Math.ceil(
      projected - input.contextWindow * CONTEXT_WINDOW_GUARD_RATIO,
    );
  }
  return overrun;
}

function exceedsGuard(projected: number, contextWindow: number): boolean {
  return projected > contextWindow * CONTEXT_WINDOW_GUARD_RATIO;
}

/**
 * Opens a new epoch when the decision says rebuild: `epoch` is the
 * previous epoch plus one, and `epochStart.reason` is the open reason.
 * `route` runs only then; when it returns a model, that model becomes
 * `epochStart.model`, `route:<reason>` becomes `epochStart.routeReason`
 * (the kernel's `model_change.reason`), and `;route:<reason>` is appended
 * to the decision reason. Its candidates are `listModels` of the current provider, and the
 * chunk accesses it sees come from the chunk index and are not copied onto
 * the manifest. Without a rebuild the epoch is unchanged and
 * `assembledStart` (a first-time epoch-head message) is kept.
 */
export function openEpoch(input: {
  baseEpoch: number;
  choice: EpochChoice;
  assembledStart: { reason: string } | undefined;
  route: RoutePolicy | undefined;
  model: ModelRef;
  budget: number;
  chunks: readonly Chunk[];
  rows: readonly ChunkRow[];
}): {
  epoch: number;
  epochStart: EpochStart | undefined;
  choice: EpochChoice;
} {
  if (!input.choice.decision.rebuild) {
    return {
      epoch: input.baseEpoch,
      epochStart: input.assembledStart,
      choice: input.choice,
    };
  }
  const epoch = input.baseEpoch + 1;
  const epochStart = { reason: openReason(input.choice) };
  const routed = input.route?.({
    epoch,
    current: input.model,
    candidates: [...listModels(input.model.provider)],
    viewChunks: routeChunks(input.chunks, input.rows),
    decision: input.choice.decision,
    budget: input.budget,
  });
  if (!routed) return { epoch, epochStart, choice: input.choice };
  const routeReason = `route:${routed.reason}`;
  return {
    epoch,
    epochStart: { ...epochStart, model: routed.model, routeReason },
    choice: appendReason(input.choice, routeReason),
  };
}

/** Appends `;<suffix>` to the decision reason; the Log text only. */
export function appendReason(choice: EpochChoice, suffix: string): EpochChoice {
  return {
    ...choice,
    decision: {
      ...choice.decision,
      reason: `${choice.decision.reason};${suffix}`,
    },
  };
}

function routeChunks(chunks: readonly Chunk[], records: readonly ChunkRow[]) {
  const byId = new Map(chunks.map((chunk) => [chunk.id, chunk]));
  return records.map((record) => {
    const chunk = byId.get(record.id);
    return {
      id: record.id,
      tier: record.tier,
      reason: record.reason,
      tokens: record.tokens,
      ...(chunk ? { accesses: chunk.accesses } : {}),
    };
  });
}
