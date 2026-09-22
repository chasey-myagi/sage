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
 * Intentionally not in the Log: the live CompiledView object. The kernel
 * appends the manifest as its own entry; this module does not.
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
};

export type EpochDecisionReport = {
  rebuild: boolean;
  oldTokens: number;
  newTokens: number;
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
  ratio: number;
};

export type CacheReport = {
  expectedReadTokens: number;
  measuredReadTokens: number;
};

export type Manifest = {
  epoch: number;
  turn: number;
  /** A finite number, or the compiler's `{ total, used, overrun }` report. */
  budget?: number | BudgetReport;
  claims: ClaimRecord[];
  chunks: ChunkRecord[];
  anomalies: Anomaly[];
  /** Cost-inequality result for this compile. Optional for older manifests. */
  epochDecision?: EpochDecisionReport;
  /** Previous request: measured input tokens over the compiler's estimate. */
  calibration?: CalibrationReport;
  /** Frozen-prefix estimate against the previous turn's cache read. */
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
   * does not store this field itself.
   */
  epochStart?: { reason: string };
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
};

export interface ViewProvider {
  view(ctx: ViewContext): Promise<CompiledView> | CompiledView;
}
