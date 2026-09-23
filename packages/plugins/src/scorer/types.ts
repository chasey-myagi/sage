/**
 * Replaceable chunk scorer: the interface every scorer implements and the
 * compiler calls.
 *
 * Guarantees: a decision tier is `none`, `summary`, or `full`. A scorer
 * reports failure as `fallback`, which tells the caller to keep the
 * deterministic tiers. Field meanings live on the types below.
 *
 * Intentionally not in the Log: the Scorer object, `requested`, and
 * `dropped`. The compiler copies decisions, usage, fallback, and `raw` onto
 * the manifest.
 */

import type { Request, Usage } from "@sage/provider/contract";
import type { Chunk } from "../compiler/chunks.js";
import type { Tier } from "../compiler/claim.js";

/** What a scorer judges: candidate chunks and the context to judge them in. */
export type ScoreInput = {
  /** Chunks the caller still treats as `kept`. */
  candidates: readonly Chunk[];
  /**
   * The previous main request when one exists. Entry and shadow scoring
   * pass this turn's compiled view instead.
   */
  prevRequest: Request | undefined;
  /** The latest user query. */
  query: string;
  /** Aborts the scorer's own call. */
  signal?: AbortSignal;
};

/** One chunk's judged tier. */
export type ScoreDecision = {
  id: string;
  tier: Tier;
  /** The model's own note for tier `summary`, at most 200 characters. */
  summary?: string;
  /** Chunk reason to store; empty or absent means `scorer:model`. */
  reason?: string;
};

/** The start of a model reply that fell back. */
export type ScoreRaw = {
  /** At most 300 characters of the reply text. */
  text: string;
  /** The reply's finish kind. */
  finish: string;
};

/** A scorer's answer. Scorers resolve failures here instead of throwing. */
export type ScoreResult = {
  decisions: ScoreDecision[];
  /** Present only when an adapter was asked. */
  usage?: Usage;
  /** Why the caller must keep the deterministic tiers; written to the Log. */
  fallback?: string;
  /**
   * `false` when the scorer fell back before sending any request; rebuild,
   * entry, and shadow reports then count 0 calls. Absent counts as one call.
   * Not written to the Log.
   */
  requested?: false;
  /** Present only when the call fell back after a model reply. */
  raw?: ScoreRaw;
  /** Items the parser discarded, including ids that were not candidates. */
  dropped?: number;
};

/** A replaceable chunk scorer the compiler calls on kept tool chunks. */
export interface Scorer {
  /** Name recorded in the Log (`manifest.shadow.scorer`). */
  readonly name: string;
  /**
   * `true` only for a scorer that makes no call and returns every candidate
   * full. The compiler then leaves tiers and reasons untouched instead of
   * rewriting them, and offline replay keeps it instead of swapping it out.
   */
  readonly isNoop?: true;
  /** Judges the candidates. Implementations resolve failures as `fallback`. */
  score(input: ScoreInput): Promise<ScoreResult>;
}
