/**
 * The shadow scorer: records what a scorer would decide without applying
 * it.
 *
 * Contract: shadow scoring runs after tiers, freeze, and budget are fixed
 * and never changes them. It never throws; any failure becomes `fallback`
 * on the record and the view still returns. `calls` counts requests
 * actually sent, with the same rule as rebuild and entry scoring.
 *
 * Intentionally not in the Log: the shadow scorer function. Its record is
 * stored on `manifest.shadow`.
 */

import {
  type CompiledView,
  readTokenCounts,
  type ViewContext,
} from "@sage/core";
import type { Message } from "@sage/provider/contract";
import type { ScoreDecision, ScoreResult, Scorer } from "../scorer/types.js";
import type { Chunk } from "./chunks.js";
import { listedChunkIds } from "./log-read.js";
import {
  failedRequestsField,
  requestCount,
  requestFromCompiled,
  SCORE_TIERS,
} from "./score-common.js";

/** `manifest.shadow`: one shadow scorer call. */
export type ShadowBody = NonNullable<CompiledView["manifest"]["shadow"]>;

/**
 * Scores tool chunks whose ids are absent from the previous manifest; on
 * the first turn every tool chunk is new. Prose chunks are not scored.
 * `prevRequest` is this turn's compiled view (head, messages, cache
 * breakpoints), not the previous turn's request. Returns undefined, and
 * makes no call, when there is no new tool chunk. `raw` is copied only
 * when the score fell back and supplied it; `failedRequests` only when
 * positive. `calls` is 0 when the scorer
 * gave up before sending (`requested: false`) or the request could not be
 * built, and 1 otherwise, including when the scorer throws.
 */
export async function runShadowScorer(input: {
  scorer: Scorer;
  chunks: readonly Chunk[];
  ctx: ViewContext;
  messages: Message[];
  cacheBreakpoints: CompiledView["cacheBreakpoints"];
  query: string;
}): Promise<ShadowBody | undefined> {
  try {
    const listed = listedChunkIds(input.ctx.path);
    const candidates = input.chunks.filter(
      (chunk) => chunk.kind === "tool" && !listed.has(chunk.id),
    );
    if (candidates.length === 0) return undefined;
    const prevRequest = requestFromCompiled(
      input.ctx,
      input.messages,
      input.cacheBreakpoints,
    );
    let result: ScoreResult;
    try {
      result = await input.scorer.score({
        candidates,
        prevRequest,
        query: input.query,
      });
    } catch (error) {
      // Where the scorer threw is unknown, so count the call as sent, as
      // entry scoring does.
      return failedShadow(input.scorer, error, 1);
    }
    const usage = readTokenCounts(result.usage);
    return {
      scorer: input.scorer.name,
      calls: requestCount(result),
      decisions: shadowDecisions(candidates, result.decisions),
      ...(usage ? { usage } : {}),
      ...(typeof result.fallback === "string"
        ? { fallback: result.fallback }
        : {}),
      ...(typeof result.fallback === "string" && result.raw
        ? { raw: result.raw }
        : {}),
      ...failedRequestsField(result),
    };
  } catch (error) {
    // Building the request or reading the Log failed; shadow data is
    // optional, so the compile must not fail with it. No request was sent.
    return failedShadow(input.scorer, error, 0);
  }
}

function failedShadow(
  scorer: Scorer,
  error: unknown,
  calls: number,
): ShadowBody {
  return {
    scorer: scorer.name,
    calls,
    decisions: [],
    fallback: shadowFailureReason(error),
  };
}

function shadowDecisions(
  candidates: readonly Chunk[],
  decisions: readonly ScoreDecision[],
): ShadowBody["decisions"] {
  const allowed = new Set(candidates.map((chunk) => chunk.id));
  const kept: ShadowBody["decisions"] = [];
  for (const decision of decisions) {
    if (!allowed.has(decision.id) || !SCORE_TIERS.has(decision.tier)) continue;
    kept.push({
      id: decision.id,
      tier: decision.tier,
      ...(decision.reason !== undefined && decision.reason.length > 0
        ? { reason: decision.reason }
        : {}),
    });
  }
  return kept;
}

/**
 * The shadow `fallback` text for a thrown value. Unlike `errorText`, it never
 * returns an empty string or "undefined": the Log field must name a reason.
 */
function shadowFailureReason(error: unknown): string {
  if (error instanceof Error && error.message.length > 0) return error.message;
  const text = String(error);
  return text.length > 0 && text !== "undefined" ? text : "error";
}
