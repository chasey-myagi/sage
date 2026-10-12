/**
 * The entry scorer: tiers each large new tool chunk once, on the turn it
 * enters the view.
 *
 * Contract: a chunk's tier is fixed when it first enters the view and the
 * cumulative freeze keeps it, so the bytes before it never change inside
 * an epoch. Every failure leaves the candidates full. `calls` counts only
 * a request actually sent: a result with `requested: false` records 0, as
 * rebuild scoring does; a throw still counts 1.
 *
 * Intentionally not in the Log: the scorer function and its request. The
 * re-tiered rows and `manifest.entryScored` are the record.
 */

import { type EntryScoredReport, readTokenCounts } from "@sage/core";
import type { Request } from "@sage/provider/contract";
import type { ScoreDecision, ScoreResult, Scorer } from "../scorer/types.js";
import { applyDecisions } from "./apply.js";
import type { TokenCounter } from "./budget.js";
import type { Chunk } from "./chunks.js";
import { EXPAND_TOOL_NAME } from "./expand.js";
import { listedChunkIds, type SessionPath } from "./log-read.js";
import { entryReason, summaryRejectedReason } from "./reasons.js";
import {
  type ChunkRow,
  firstById,
  keptRow,
  rowsToDecisions,
  stubRow,
  summaryRow,
} from "./rows.js";
import {
  clampSummary,
  failedRequestsField,
  isLocatableSummary,
  requestCount,
  SCORE_TIERS,
} from "./score-common.js";
import type { ScoredView } from "./score-rebuild.js";

/**
 * Default `minScoreTokens`: a new tool chunk smaller than this many
 * estimated tokens is not worth an entry-scorer call and stays full.
 */
export const DEFAULT_MIN_SCORE_TOKENS = 300;

/**
 * Returns `value` when it is a finite number ≥ 0, else
 * `DEFAULT_MIN_SCORE_TOKENS`.
 */
export function resolveMinScoreTokens(value: number | undefined): number {
  if (typeof value === "number" && Number.isFinite(value) && value >= 0) {
    return value;
  }
  return DEFAULT_MIN_SCORE_TOKENS;
}

/**
 * Scores this turn's new tool chunks in one call and applies the result.
 *
 * Candidates are tool chunks absent from the previous manifest, still at
 * tier full, at least `minScoreTokens`, and not `expand` results. `none`
 * becomes a stub. `summary` keeps the note cut to 200 characters; a missing
 * note leaves the chunk as it was. A `stubOnly` summary shows the stub
 * alone and is not checked for a note. A note shorter than 20 characters, or
 * with no path, symbol, or number, stays full with reason
 * `scorer:entry:full;summary-rejected` and counts in both `full` and
 * `summaryRejected`. `full` keeps the text. The reason is
 * `scorer:entry:<tier>`, followed by `;<decision reason>` when the scorer
 * gave one (jev's probabilities). A throw, a fallback, or an id the reply
 * omits leaves that chunk full; the scorer's `failedRequests` is copied
 * onto the report when positive. `report` is set whenever the scorer was
 * invoked; its `calls` is 0 when the result has `requested: false`.
 * `prevRequest` is this turn's compiled view.
 */
export async function runEntryScorer(input: {
  scorer: Scorer;
  chunks: Chunk[];
  view: ScoredView;
  count: TokenCounter;
  minScoreTokens: number;
  path: SessionPath;
  prevRequest: Request;
  query: string;
}): Promise<ScoredView & { report?: EntryScoredReport }> {
  const unchanged = { ...input.view };
  const candidates = entryCandidates(input);
  if (candidates.length === 0) return unchanged;
  let result: ScoreResult;
  try {
    result = await input.scorer.score({
      candidates,
      prevRequest: input.prevRequest,
      query: input.query,
    });
  } catch {
    // A scorer defect leaves every candidate full; the call still counts.
    return { ...unchanged, report: entryReport(1, NO_TIERS) };
  }
  if (result.fallback !== undefined) {
    return {
      ...unchanged,
      report: entryReport(requestCount(result), NO_TIERS, result),
    };
  }
  const accepted = acceptedDecisions(candidates, result.decisions);
  const applied = applyEntryDecisions(input, accepted);
  return {
    rows: applied.rows,
    messages: applyDecisions(
      input.view.messages,
      input.chunks,
      rowsToDecisions(applied.rows),
    ),
    used: input.view.used + applied.usedDelta,
    report: entryReport(requestCount(result), applied, result),
  };
}

function entryCandidates(input: {
  chunks: readonly Chunk[];
  view: ScoredView;
  minScoreTokens: number;
  path: SessionPath;
}): Chunk[] {
  const listed = listedChunkIds(input.path);
  const rowById = firstById(input.view.rows);
  return input.chunks.filter((chunk) => {
    if (chunk.kind !== "tool" || chunk.toolName === EXPAND_TOOL_NAME)
      return false;
    if (listed.has(chunk.id) || chunk.tokens < input.minScoreTokens) {
      return false;
    }
    return rowById.get(chunk.id)?.tier === "full";
  });
}

/** First decision per candidate id with a tier a scorer may assign. */
function acceptedDecisions(
  candidates: readonly Chunk[],
  decisions: readonly ScoreDecision[],
): Map<string, ScoreDecision> {
  const allowed = new Set(candidates.map((chunk) => chunk.id));
  const byId = new Map<string, ScoreDecision>();
  for (const decision of decisions) {
    if (!allowed.has(decision.id) || byId.has(decision.id)) continue;
    if (!SCORE_TIERS.has(decision.tier)) continue;
    byId.set(decision.id, decision);
  }
  return byId;
}

type EntryApplied = {
  rows: ChunkRow[];
  usedDelta: number;
  none: number;
  summary: number;
  full: number;
  summaryRejected: number;
};

function applyEntryDecisions(
  input: { chunks: readonly Chunk[]; view: ScoredView; count: TokenCounter },
  accepted: ReadonlyMap<string, ScoreDecision>,
): EntryApplied {
  const chunksById = new Map(input.chunks.map((chunk) => [chunk.id, chunk]));
  const tally = { usedDelta: 0, none: 0, summary: 0, full: 0, rejected: 0 };
  const rows = input.view.rows.map((row) => {
    const scored = accepted.get(row.id);
    const chunk = chunksById.get(row.id);
    if (!scored || chunk?.kind !== "tool") return row;
    const reason = entryReason(row.reason, scored.tier, scored.reason);
    if (scored.tier === "full") {
      tally.full += 1;
      return keptRow(row.id, "full", reason, chunk.tokens);
    }
    if (scored.tier === "none") {
      tally.none += 1;
      tally.usedDelta -= chunk.tokens;
      return stubRow(chunk, reason, input.count);
    }
    if (scored.stubOnly) {
      tally.summary += 1;
      const stubbed = summaryRow(chunk, reason, "", input.count);
      tally.usedDelta += stubbed.tokens - chunk.tokens;
      return stubbed;
    }
    if (scored.summary === undefined) return row;
    const note = clampSummary(scored.summary);
    if (!isLocatableSummary(note)) {
      tally.full += 1;
      tally.rejected += 1;
      return keptRow(
        row.id,
        "full",
        summaryRejectedReason(row.reason, scored.reason),
        chunk.tokens,
      );
    }
    tally.summary += 1;
    const summarized = summaryRow(chunk, reason, note, input.count);
    tally.usedDelta += summarized.tokens - chunk.tokens;
    return summarized;
  });
  return {
    rows,
    usedDelta: tally.usedDelta,
    none: tally.none,
    summary: tally.summary,
    full: tally.full,
    summaryRejected: tally.rejected,
  };
}

type TierCounts = Pick<
  EntryScoredReport,
  "none" | "summary" | "full" | "summaryRejected"
>;

const NO_TIERS: TierCounts = {
  none: 0,
  summary: 0,
  full: 0,
  summaryRejected: 0,
};

function entryReport(
  calls: number,
  tiers: TierCounts,
  result?: Pick<ScoreResult, "usage" | "failedRequests">,
): EntryScoredReport {
  const counts = readTokenCounts(result?.usage);
  return {
    calls,
    none: tiers.none,
    summary: tiers.summary,
    full: tiers.full,
    summaryRejected: tiers.summaryRejected,
    ...(counts ? { usage: counts } : {}),
    ...failedRequestsField(result),
  };
}
