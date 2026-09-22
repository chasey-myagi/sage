/**
 * Replaceable chunk scorer.
 *
 * Contract: `score` receives the chunks the caller still treats as `kept`,
 * the previous main request when one exists, and the latest user query.
 * A decision tier is `none`, `summary`, or `full`. `summary` is the model's
 * own note, at most 200 characters. `reason`, when non-empty, is the chunk
 * reason the compiler stores; otherwise the compiler writes `scorer:model`.
 * `fallback` tells the caller to keep the deterministic tiers. `usage` is
 * present only when an adapter was asked.
 *
 * Intentionally not in the Log: the Scorer object. The compiler copies
 * decisions, usage, and fallback onto the manifest.
 */

import type { Request, Usage } from "@sage/provider/contract";
import type { Chunk } from "../compiler/chunks.js";
import type { Tier } from "../compiler/claim.js";

export type ScoreInput = {
  candidates: readonly Chunk[];
  prevRequest: Request | undefined;
  query: string;
  signal?: AbortSignal;
};

export type ScoreDecision = {
  id: string;
  tier: Tier;
  summary?: string;
  reason?: string;
};

export interface Scorer {
  readonly name: string;
  score(
    input: ScoreInput,
  ): Promise<{ decisions: ScoreDecision[]; usage?: Usage; fallback?: string }>;
}
