/**
 * The threshold sweep of `sage calibrate-jev`: replays Jev's cached
 * triage answers through the production tier policy (`decideTier`) under
 * every combination of a threshold grid and scores each against one
 * later-use label. Read probes decide nothing and are not swept.
 *
 * Contract: pure; rows come out in grid order (`full`, then `low`,
 * `keep`, each ascending). A chunk is omitted when its tier is `summary` or
 * `none`: either way the original text leaves the view. Costs are in
 * input-price-equivalent tokens under the compiler's `ratesFor`, so
 * sessions priced in different currencies, or not priced at all, add up.
 *
 * Intentionally not in the Log: the sweep. It is computed from the Log and
 * cached answers, and nothing reads it back.
 */

import {
  DEFAULT_JEV_THRESHOLDS,
  decideTier,
  type JevThresholds,
  type Rates,
  type TriageJudgment,
} from "@sage/plugins";

/** One triaged chunk with its label and the facts its costs need. */
export type LabeledChunk = {
  /** Basename of the session file; chunk ids repeat across sessions. */
  session: string;
  chunkId: string;
  /** The compiler's token estimate of the whole chunk. */
  tokens: number;
  /** Turns with a reply after the chunk's entry turn. */
  remainingTurns: number;
  /**
   * Needed after its entry turn under the label being swept: `touch` or
   * `content` of the `LaterUse` labels.
   */
  needed: boolean;
  /** `ratesFor` of the entry turn's `request_header.pricing`. */
  rates: Rates;
  judgment: TriageJudgment;
  /** The candidate lines `judgment.keep` indexes. */
  candidates: readonly string[];
};

/** Values to try per threshold; every combination is one row. */
export type SweepGrid = {
  full: readonly number[];
  low: readonly number[];
  keep: readonly number[];
};

/**
 * Default grid: brackets each `DEFAULT_JEV_THRESHOLDS` value with steps of
 * 0.1, 45 combinations in all.
 */
export const DEFAULT_SWEEP_GRID: Readonly<SweepGrid> = {
  full: [0.2, 0.3, 0.4, 0.5, 0.6],
  low: [0.1, 0.2, 0.3],
  keep: [0.3, 0.5, 0.7],
};

/** One threshold combination's outcome over every labeled chunk. */
export type SweepRow = {
  thresholds: JevThresholds;
  tiers: { full: number; summary: number; none: number };
  /** Chunks at `summary` or `none`. */
  omitted: number;
  /** Chunks needed later, whatever their tier. */
  needed: number;
  /** Omitted chunks that were needed later. */
  omittedNeeded: number;
  /** `omittedNeeded / needed`; null when no chunk was needed. */
  missRate: number | null;
  /**
   * Omitted chunks never needed later, over all omitted; null when none
   * was omitted.
   */
  precision: number | null;
  /**
   * Omitted chunks never needed later, over all chunks never needed;
   * null when every chunk was needed.
   */
  recall: number | null;
  /**
   * Expected cost in input-price-equivalent tokens: each omitted chunk
   * that was needed pays an expand, `tokens × rates.write`, since it
   * re-enters as a cache write; each full chunk never needed pays
   * `tokens × rates.read × remainingTurns` for being re-read on every
   * later turn. Omitted chunks not needed and full chunks that were
   * needed cost 0.
   */
  cost: number;
};

/** Scores every combination of `grid` over `chunks`, in grid order. */
export function sweepThresholds(
  chunks: readonly LabeledChunk[],
  grid: SweepGrid,
): SweepRow[] {
  const rows: SweepRow[] = [];
  for (const full of grid.full) {
    for (const low of grid.low) {
      for (const keep of grid.keep) {
        rows.push(scoreRow(chunks, { full, low, keep }));
      }
    }
  }
  return rows;
}

/**
 * The row with the lowest cost; among equal costs, the one closest to
 * `DEFAULT_JEV_THRESHOLDS` (Euclidean over the three values), then the
 * earliest in grid order. Undefined for no rows.
 */
export function recommendRow(rows: readonly SweepRow[]): SweepRow | undefined {
  let best: SweepRow | undefined;
  for (const row of rows) {
    if (
      !best ||
      row.cost < best.cost ||
      (row.cost === best.cost &&
        distanceToDefault(row.thresholds) < distanceToDefault(best.thresholds))
    ) {
      best = row;
    }
  }
  return best;
}

function distanceToDefault(thresholds: JevThresholds): number {
  return Math.hypot(
    thresholds.full - DEFAULT_JEV_THRESHOLDS.full,
    thresholds.low - DEFAULT_JEV_THRESHOLDS.low,
    thresholds.keep - DEFAULT_JEV_THRESHOLDS.keep,
  );
}

function scoreRow(
  chunks: readonly LabeledChunk[],
  thresholds: JevThresholds,
): SweepRow {
  const tiers = { full: 0, summary: 0, none: 0 };
  const tally = { needed: 0, omittedNeeded: 0, omittedUnneeded: 0, cost: 0 };
  for (const chunk of chunks) {
    const { tier } = decideTier(
      chunk.chunkId,
      chunk.judgment,
      chunk.candidates,
      thresholds,
    );
    if (tier === "full" || tier === "summary" || tier === "none") {
      tiers[tier] += 1;
    }
    const omitted = tier === "summary" || tier === "none";
    if (chunk.needed) tally.needed += 1;
    if (omitted && chunk.needed) {
      tally.omittedNeeded += 1;
      tally.cost += chunk.tokens * chunk.rates.write;
    } else if (omitted) {
      tally.omittedUnneeded += 1;
    } else if (!chunk.needed) {
      tally.cost += chunk.tokens * chunk.rates.read * chunk.remainingTurns;
    }
  }
  const omitted = tiers.summary + tiers.none;
  const unneeded = chunks.length - tally.needed;
  return {
    thresholds,
    tiers,
    omitted,
    needed: tally.needed,
    omittedNeeded: tally.omittedNeeded,
    missRate: tally.needed === 0 ? null : tally.omittedNeeded / tally.needed,
    precision: omitted === 0 ? null : tally.omittedUnneeded / omitted,
    recall: unneeded === 0 ? null : tally.omittedUnneeded / unneeded,
    cost: tally.cost,
  };
}
