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
