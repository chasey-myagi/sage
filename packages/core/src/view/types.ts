/**
 * View seam types: what a ViewProvider receives (`ViewContext`) and returns
 * (`CompiledView`), and the manifest the kernel logs for each request.
 * Domain terms (chunk, tier, claim, epoch, stub, manifest) are defined on
 * the types below.
 *
 * Guarantees: a CompiledView is the only model-facing projection for one
 * call. The kernel checks pairing and the frozen head, then logs the
 * envelope; it does not score, drop, or rewrite blocks, and it stores the
 * manifest's scorer reports without reading or applying them.
 *
 * Intentionally not in the Log: the live CompiledView, `epochStart` (the
 * kernel turns it into a `view_epoch` entry), `parentPath`, and `depth`.
 * The kernel appends the manifest as its own entry; this module does not.
 * A child session's `subtask` entry is written by the caller.
 */

import type {
  CacheBreakpoint,
  Head,
  Message,
  ModelRef,
  PreparedCall,
  TokenCounts,
} from "@sage/provider/contract";
import type { StoredEntry } from "../log/types.js";
import type { HarnessTool } from "../tools.js";

/**
 * Tier: the form a chunk or claim takes in the view. `full` is the original
 * text; `summary` a short note in its place; `line` a single line; `none`
 * leaves only a stub, one line that names the omitted chunk, its size, and
 * how to `expand` it back.
 */
export type Tier = "none" | "line" | "summary" | "full";

/**
 * One manifest line: the tier chosen for one chunk or claim and why.
 * `reason` is for people and the Log, never for branching. `tokens` is the
 * estimated size at that tier (for `none`, the stub's size).
 */
export type TierRecord = {
  id: string;
  tier: Tier;
  reason: string;
  tokens?: number;
};

/**
 * Tier record for a chunk: the smallest unit the compiler schedules, one
 * tool call plus its result (or one plain message), always kept or dropped
 * whole. Same fields as `ClaimRecord`; the name says which list it is in.
 */
export type ChunkRecord = TierRecord;

/**
 * Tier record for a claim: a plugin's request for view space (who it is,
 * how much it needs, which tiers it can live with). The compiler admits
 * text only through claims. Same fields as `ChunkRecord`.
 */
export type ClaimRecord = TierRecord;

/** A pairing or content problem the view repaired, recorded on the manifest. */
export type Anomaly = {
  kind: "dangling_tool_use" | "orphan_tool_result" | "empty_message";
  id?: string;
  note?: string;
};

/** The compiler's token budget for this view. Fields are tokens unless noted. */
export type BudgetReport = {
  total: number;
  used: number;
  overrun: number;
  /** Kept tokens of chunks whose tier was copied from this epoch's freeze. */
  frozenTokens?: number;
  /** Subset of `frozenTokens` a from-scratch eliminate would set to none. */
  staleOmittable?: number;
  /** Chunks at tier `none` in this view, each shown as a stub. A count. */
  omitted?: number;
  /**
   * `expand` tool calls in assistant messages on the path so far: a running
   * count, not this turn's. `runHealth` reads it with `omitted` for the
   * expand rate.
   */
  expands?: number;
};

/**
 * Whether this compile opens a new epoch, and the cost inequality behind it.
 * Opening an epoch rewrites the frozen prefix: the new view is paid at the
 * cache-write rate once, then every later turn reads fewer tokens.
 * `readRate` and `writeRate` are cache read and write prices relative to
 * the input price. `savingsPerTurn` is `(oldTokens - newTokens) * readRate`;
 * `extraCost` is `newTokens * writeRate - oldTokens * readRate`.
 * `paybackTurns` (payback) is `ceil(extraCost / savingsPerTurn)`: 0 when
 * `extraCost` is not positive, null when there are no savings. `rebuild`
 * is true when payback fits within `remainingTurns`, or for a guard reason.
 */
export type EpochDecisionReport = {
  rebuild: boolean;
  oldTokens: number;
  newTokens: number;
  /**
   * Horizon the inequality compared against payback. The compiler sets it to
   * `max(1, expectedTurns - started, ceil(alpha * started))`: `expectedTurns`
   * is the assumed task length, `started` the turns already begun, and
   * `alpha` the fraction of `started` still assumed to lie ahead, so a long
   * run keeps a horizon that grows with it.
   */
  remainingTurns: number;
  readRate: number;
  writeRate: number;
  savingsPerTurn: number;
  extraCost: number;
  paybackTurns: number | null;
  reason: string;
};

/** Token estimate against the provider's measured input for the last request. */
export type CalibrationReport = {
  measured: number;
  estimated: number;
  /**
   * Fixed overhead in tokens (not the system+tools `Head`): the epoch's
   * first measured input minus that turn's budget. Persisted under this
   * name; see docs/notes/proposed/2026-09-24-head-naming.md.
   */
  head: number;
  ratio: number;
};

/** Expected against measured cache-read tokens for the previous request. */
export type CacheReport = {
  expectedReadTokens: number;
  measuredReadTokens: number;
};

/** One shadow decision: the tier a shadow scorer would have chosen. */
export type ShadowDecisionRecord = {
  id: string;
  tier: Tier;
  reason?: string;
};

/** Model text prefix and finish kind. Written only when a score falls back. */
export type ScorerRaw = {
  text: string;
  finish: string;
};

/**
 * Shadow scoring: a scorer judges the tool chunks that first appeared this
 * turn after the view is final, and only this record is kept. The tiers
 * are never applied, so the view and its prefix are unchanged; the record
 * exists to measure scorer accuracy afterwards.
 */
export type ShadowReport = {
  scorer: string;
  decisions: ShadowDecisionRecord[];
  usage?: TokenCounts;
  fallback?: string;
  raw?: ScorerRaw;
};

/** Scorer calls made while rebuilding an epoch. `raw` only on fallback. */
export type ScorerReport = {
  calls: number;
  usage: TokenCounts;
  fallback: boolean;
  raw?: ScorerRaw;
};

/**
 * Entry scoring: one scorer call that sets the tier of each tool chunk the
 * first time it enters the view, and the tier is then frozen, so scoring
 * never rewrites bytes already in the prefix (unlike shadow scoring, the
 * tiers are applied). `calls` is 1 even when the scorer throws or omits an
 * id. Tier counts are decisions that were applied. `summaryRejected`
 * counts summary notes demoted to full; those chunks are included in
 * `full`. `usage` is omitted when the call reported none. An applied
 * decision's chunk reason is `scorer:entry:<tier>`; a rejected summary's is
 * `scorer:entry:full;summary-rejected`.
 */
export type EntryScoredReport = {
  calls: number;
  none: number;
  summary: number;
  full: number;
  summaryRejected: number;
  usage?: TokenCounts;
};

/**
 * Manifest: the compiler's account of one view, appended by the kernel as a
 * `manifest` entry before the request is sent. Every chunk and claim in the
 * view has a line with a tier and a reason. The kernel stores the scorer
 * reports and does not read or apply them.
 */
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
   * fixed overhead `head`, and `(measured − head) / estimated`.
   */
  calibration?: CalibrationReport;
  /**
   * Previous turn's frozen prefix (system and tools + frozen chunks +
   * claims in the epoch's first message) against that turn's cache read.
   * A large gap means the prefix was rewritten.
   */
  cache?: CacheReport;
  shadow?: ShadowReport;
  scorer?: ScorerReport;
  /** Already applied to `chunks`; this object is the call's counts. */
  entryScored?: EntryScoredReport;
};

/**
 * The model-facing projection for one call.
 *
 * `epoch` numbers the current epoch: a stretch of turns whose prefix (head
 * plus the frozen start of `messages`) stays byte-identical so the
 * provider's prefix cache keeps hitting; within it the view only appends.
 * A new epoch recomputes the prefix once.
 */
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
   * Opaque bag copied onto this turn's manifest entry. Claim text lives
   * here under `claimContent`, and only for hashes that are new this turn.
   */
  details?: Record<string, unknown>;
};

/** What the session hands a ViewProvider each turn. */
export type ViewContext = {
  sessionId: string;
  turn: number;
  /** Structured clone of the session path, taken before `view()`. */
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

/**
 * The View seam: turns a session path into this turn's `CompiledView`.
 * May be async. A throw ends the run with a terminal error.
 */
export interface ViewProvider {
  view(ctx: ViewContext): Promise<CompiledView> | CompiledView;
}
