/**
 * View seam shapes.
 *
 * Contract: a CompiledView is the only model-facing projection for one call.
 * The kernel checks pairing and epoch freeze, then logs the envelope. It does
 * not score, drop, or rewrite blocks.
 *
 * `epochStart` is a signal, not a log field: the kernel turns it into a
 * `view_epoch` entry. `details` is copied onto that turn's manifest entry.
 *
 * `parentPath` is a snapshot of another session's chain. `depth` is 0 on a
 * root session. Neither is stored by the kernel.
 *
 * `shadow` on a manifest is the shadow scorer's record for this turn. The
 * kernel stores it with the manifest and does not apply those tiers.
 * `shadow.raw` and `scorer.raw`, when present, are the model text prefix
 * and finish kind from a fallback. A successful score omits `raw`.
 * `entryScored`, when present, is one entry-mode scorer call: how many
 * candidates became `none`, `summary`, or `full`, how many summary notes
 * were rejected, plus that call's usage. Chunk `reason` for an applied
 * entry decision is `scorer:entry:<tier>`. A rejected summary uses
 * `scorer:entry:full;summary-rejected`.
 * The kernel stores the object and does not read those counts.
 *
 * Intentionally not in the Log: the live CompiledView object, parentPath,
 * and depth. The kernel appends the manifest as its own entry; this module
 * does not. A child session's `subtask` entry is written by the caller.
 */

import type {
  CacheBreakpoint,
  Head,
  Message,
  ModelRef,
  PreparedCall,
} from "@sage/provider/contract";
import type { StoredEntry } from "../log/types.js";
import type { HarnessTool } from "../tools.js";

export type ChunkRecord = {
  id: string;
  tier: "none" | "line" | "summary" | "full";
  reason: string;
  tokens?: number;
};

export type ClaimRecord = {
  id: string;
  tier: ChunkRecord["tier"];
  reason: string;
  tokens?: number;
};

export type Anomaly = {
  kind: "dangling_tool_use" | "orphan_tool_result" | "empty_message";
  id?: string;
  note?: string;
};

export type BudgetReport = {
  total: number;
  used: number;
  overrun: number;
  /** Kept tokens of chunks whose tier was copied from this epoch's freeze. */
  frozenTokens?: number;
  /** Subset of `frozenTokens` a from-scratch eliminate would set to none. */
  staleOmittable?: number;
};

export type EpochDecisionReport = {
  rebuild: boolean;
  oldTokens: number;
  newTokens: number;
  /**
   * Horizon the inequality compared against payback. The compiler sets it to
   * max(1, expectedTurns - started, ceil(alpha * started)).
   */
  remainingTurns: number;
  readRate: number;
  writeRate: number;
  savingsPerTurn: number;
  extraCost: number;
  paybackTurns: number | null;
  reason: string;
};

export type CalibrationReport = {
  measured: number;
  estimated: number;
  /** Fixed overhead: first measured input of the epoch minus that turn's budget. */
  head: number;
  ratio: number;
};

export type CacheReport = {
  expectedReadTokens: number;
  measuredReadTokens: number;
};

export type ShadowDecisionRecord = {
  id: string;
  tier: ChunkRecord["tier"];
  reason?: string;
};

/** Model text prefix and finish kind. Written only when a score falls back. */
export type ScorerRaw = {
  text: string;
  finish: string;
};

/**
 * Shadow score for tool chunks that first appeared this turn. The compiler
 * records it and does not apply the tiers.
 */
export type ShadowReport = {
  scorer: string;
  decisions: ShadowDecisionRecord[];
  usage?: {
    inputOther: number;
    inputCacheRead: number;
    inputCacheCreation: number;
    output: number;
  };
  fallback?: string;
  raw?: ScorerRaw;
};

export type ScorerReport = {
  calls: number;
  usage: {
    inputOther: number;
    inputCacheRead: number;
    inputCacheCreation: number;
    output: number;
  };
  fallback: boolean;
  raw?: ScorerRaw;
};

/**
 * One entry-mode score. `calls` is 1 even when the scorer throws or omits
 * an id. Tier counts are decisions that were applied. `summaryRejected`
 * counts summary notes demoted to full; those chunks are included in
 * `full`. `usage` is the provider quadruple, omitted when the call
 * reported none.
 */
export type EntryScoredReport = {
  calls: number;
  none: number;
  summary: number;
  full: number;
  summaryRejected: number;
  usage?: {
    inputOther: number;
    inputCacheRead: number;
    inputCacheCreation: number;
    output: number;
  };
};

export type Manifest = {
  epoch: number;
  turn: number;
  /**
   * A finite number, or the compiler's budget report. The kernel does not
   * read `frozenTokens` or `staleOmittable`.
   */
  budget?: number | BudgetReport;
  claims: ClaimRecord[];
  chunks: ChunkRecord[];
  anomalies: Anomaly[];
  /** Cost-inequality result for this compile. Optional for older manifests. */
  epochDecision?: EpochDecisionReport;
  /**
   * Previous request: measured input, the compiler's `budget.used`, the
   * fixed head, and `(measured − head) / estimated`.
   */
  calibration?: CalibrationReport;
  /**
   * Previous turn's frozen prefix (`head` + frozen chunks + epoch-head
   * claims) against that turn's cache read. A large gap means the prefix
   * was rewritten.
   */
  cache?: CacheReport;
  /**
   * Shadow score of this turn's new tool chunks. Optional. The kernel stores
   * the object and does not apply it to chunk tiers.
   */
  shadow?: ShadowReport;
  /**
   * Rebuild scorer record. Optional. `raw` is present only on fallback.
   */
  scorer?: ScorerReport;
  /**
   * Entry-mode score for this turn's new tool chunks. Optional. The tiers
   * are already applied to `chunks`; this object is the call count.
   */
  entryScored?: EntryScoredReport;
};

export type CompiledView = {
  epoch: number;
  head: Head;
  messages: Message[];
  cacheBreakpoints: CacheBreakpoint[];
  manifest: Manifest;
  /**
   * Set when this view opens an epoch. The kernel appends `view_epoch` and
   * does not store this field itself. `model`, when set and different from
   * the session's current model, becomes a `model_change` entry first.
   */
  epochStart?: { reason: string; model?: ModelRef };
  /**
   * Opaque bag copied onto the manifest entry. Claim text lives here under
   * `claimContent`, and only for hashes that are new this turn.
   */
  details?: Record<string, unknown>;
};

export type ViewContext = {
  sessionId: string;
  turn: number;
  path: readonly StoredEntry[];
  model: ModelRef;
  prepared: PreparedCall;
  /** Session-injected head for this turn. Replay reads it back off the log. */
  head: Head;
  /** Tools registered on the session. Passthrough does not read them. */
  tools: readonly HarnessTool[];
  /**
   * Parent chain frozen at `parent.leafId`. Set only for a child session.
   * The compiler reads it and does not append those entries here.
   */
  parentPath?: readonly StoredEntry[];
  /** 0 for a root session. A child is its parent's depth plus one. */
  depth: number;
};

export interface ViewProvider {
  view(ctx: ViewContext): Promise<CompiledView> | CompiledView;
}
