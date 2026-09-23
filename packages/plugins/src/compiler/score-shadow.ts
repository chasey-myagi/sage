/**
 * The shadow scorer: records what a scorer would decide without applying
 * it.
 *
 * Contract: shadow scoring runs after tiers, freeze, and budget are fixed
 * and never changes them. It never throws; any failure becomes `fallback`
 * on the record and the view still returns.
 *
 * Intentionally not in the Log: the shadow scorer function. Its record is
 * stored on `manifest.shadow`.
 */

import type { CompiledView, ViewContext } from "@sage/core";
import type { Message, Usage } from "@sage/provider/contract";
import type { ScoreDecision, ScoreRaw, Scorer } from "../scorer/types.js";
import type { Chunk } from "./chunks.js";
import { listedChunkIds } from "./log-read.js";
import {
  finiteUsage,
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
 * when the score fell back and supplied it.
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
    let result: {
      decisions: ScoreDecision[];
      usage?: Usage;
      fallback?: string;
      raw?: ScoreRaw;
    };
    try {
      result = await input.scorer.score({
        candidates,
        prevRequest: requestFromCompiled(
          input.ctx,
          input.messages,
          input.cacheBreakpoints,
        ),
        query: input.query,
      });
    } catch (error) {
      return failedShadow(input.scorer, error);
    }
    const usage = result.usage ? finiteUsage(result.usage) : undefined;
    return {
      scorer: input.scorer.name,
      decisions: shadowDecisions(candidates, result.decisions),
      ...(usage ? { usage } : {}),
      ...(typeof result.fallback === "string"
        ? { fallback: result.fallback }
        : {}),
      ...(typeof result.fallback === "string" && result.raw
        ? { raw: result.raw }
        : {}),
    };
  } catch (error) {
    // Building the request or reading the Log failed; shadow data is
    // optional, so the compile must not fail with it.
    return failedShadow(input.scorer, error);
  }
}

function failedShadow(scorer: Scorer, error: unknown): ShadowBody {
  return { scorer: scorer.name, decisions: [], fallback: errorText(error) };
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

function errorText(error: unknown): string {
  if (error instanceof Error && error.message.length > 0) return error.message;
  const text = String(error);
  return text.length > 0 && text !== "undefined" ? text : "error";
}
