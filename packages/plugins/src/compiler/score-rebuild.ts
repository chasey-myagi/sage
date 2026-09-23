/**
 * The rebuild scorer: re-tiers kept tool chunks when the compile opens a
 * new epoch.
 *
 * Contract: it runs only on a rebuild, and only when at least one tool
 * chunk still has reason `kept`; recent, superseded, retry, and budget
 * decisions always win. A scorer with `isNoop` (the rule scorer) leaves
 * every tier and reason untouched. A scorer failure keeps the eliminate tiers and is reported,
 * never thrown.
 *
 * Intentionally not in the Log: the scorer function and its request. The
 * re-tiered rows and `manifest.scorer` are the record.
 */

import {
  emptyUsage,
  type Message,
  type Request,
  type Usage,
} from "@sage/provider/contract";
import type {
  ScoreDecision,
  ScoreRaw,
  ScoreResult,
  Scorer,
} from "../scorer/types.js";
import { applyDecisions } from "./apply.js";
import type { TokenCounter } from "./budget.js";
import type { Chunk } from "./chunks.js";
import { modelReason, reasonKind } from "./reasons.js";
import {
  type ChunkRow,
  keptRow,
  rowsToDecisions,
  stubRow,
  summaryRow,
} from "./rows.js";
import { clampSummary, requestCount, SCORE_TIERS } from "./score-common.js";

/** `manifest.scorer`: one rebuild scorer call. */
export type RebuildScorerReport = {
  /** 0 when the scorer fell back before making a request. */
  calls: number;
  usage: Usage;
  fallback: boolean;
  /** The model's reply prefix; present only on a fallback that supplied it. */
  raw?: ScoreRaw;
};

/** Rows, messages, and token total of the chosen view. */
export type ScoredView = {
  rows: ChunkRow[];
  messages: Message[];
  used: number;
};

/**
 * Scores the kept tool chunks of the chosen view and applies the result.
 *
 * A scorer without `isNoop` may set each candidate to none, summary, or full. The
 * chunk reason becomes the decision's non-empty `reason`, else
 * `scorer:model`, keeping the previous suffixes. A summary's tool_result is
 * the stub, a newline, and `summary (model notes, not evidence): ` plus the
 * note, which is also stored on the row. A throw or fallback keeps the
 * input unchanged and returns `fallback` for the caller to append as
 * `;scorer:fallback:<reason>`. `report` is set when the call reported
 * usage or a fallback. `prevRequest` is the previous turn's request
 * rebuilt from the Log.
 */
export async function runRebuildScorer(input: {
  scorer: Scorer;
  chunks: Chunk[];
  view: ScoredView;
  count: TokenCounter;
  prevRequest: Request | undefined;
  query: string;
}): Promise<ScoredView & { fallback?: string; report?: RebuildScorerReport }> {
  const { view } = input;
  const candidates = keptToolChunks(view.rows, input.chunks);
  if (candidates.length === 0) return { ...view };
  let result: ScoreResult;
  try {
    result = await input.scorer.score({
      candidates,
      prevRequest: input.prevRequest,
      query: input.query,
    });
  } catch {
    // A scorer defect degrades to the eliminate tiers; the compile goes on.
    result = { decisions: [], fallback: "error" };
  }
  const report = reportOf(result);
  if (result.fallback !== undefined || input.scorer.isNoop) {
    return {
      ...view,
      ...(result.fallback !== undefined ? { fallback: result.fallback } : {}),
      ...(report ? { report } : {}),
    };
  }
  const merged = mergeScore(
    view.rows,
    input.chunks,
    result.decisions,
    input.count,
  );
  return {
    rows: merged.rows,
    messages: applyDecisions(
      view.messages,
      input.chunks,
      rowsToDecisions(merged.rows),
    ),
    used: view.used + merged.usedDelta,
    ...(report ? { report } : {}),
  };
}

function keptToolChunks(
  rows: readonly ChunkRow[],
  chunks: readonly Chunk[],
): Chunk[] {
  const byId = new Map(chunks.map((chunk) => [chunk.id, chunk]));
  const kept: Chunk[] = [];
  for (const row of rows) {
    if (reasonKind(row.reason) !== "kept") continue;
    const chunk = byId.get(row.id);
    if (chunk?.kind !== "tool") continue;
    kept.push(chunk);
  }
  return kept;
}

function mergeScore(
  rows: readonly ChunkRow[],
  chunks: readonly Chunk[],
  decisions: readonly ScoreDecision[],
  count: TokenCounter,
): { rows: ChunkRow[]; usedDelta: number } {
  const byId = new Map(decisions.map((decision) => [decision.id, decision]));
  const chunksById = new Map(chunks.map((chunk) => [chunk.id, chunk]));
  let usedDelta = 0;
  const next = rows.map((row) => {
    if (reasonKind(row.reason) !== "kept") return row;
    const chunk = chunksById.get(row.id);
    if (chunk?.kind !== "tool") return row;
    const scored = byId.get(row.id);
    if (!scored || !SCORE_TIERS.has(scored.tier)) return row;
    const reason = modelReason(row.reason, scored);
    if (scored.tier === "full") {
      return keptRow(row.id, "full", reason, chunk.tokens);
    }
    if (scored.tier === "none") {
      usedDelta -= chunk.tokens;
      return stubRow(chunk, reason, count);
    }
    const summarized = summaryRow(
      chunk,
      reason,
      clampSummary(scored.summary ?? ""),
      count,
    );
    usedDelta += summarized.tokens - chunk.tokens;
    return summarized;
  });
  return { rows: next, usedDelta };
}

/** `calls` comes from `requestCount`. */
function reportOf(result: ScoreResult): RebuildScorerReport | undefined {
  if (result.usage === undefined && result.fallback === undefined)
    return undefined;
  const fallback = result.fallback !== undefined;
  return {
    calls: requestCount(result),
    usage: result.usage ?? emptyUsage(),
    fallback,
    ...(fallback && result.raw ? { raw: result.raw } : {}),
  };
}
