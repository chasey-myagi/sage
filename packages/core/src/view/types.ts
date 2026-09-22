/**
 * View seam shapes.
 *
 * Contract: a CompiledView is the only model-facing projection for one call.
 * The kernel checks pairing and epoch freeze, then logs the envelope. It does
 * not score, drop, or rewrite blocks.
 *
 * Intentionally not in the Log: the live CompiledView object. Manifest rows
 * are the view's own record; the kernel does not append them.
 */

import type {
  CacheBreakpoint,
  Head,
  Message,
  ModelRef,
  PreparedCall,
} from "@sage/provider/contract";
import type { StoredEntry } from "../log/types.js";

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

export type Manifest = {
  epoch: number;
  turn: number;
  budget?: number;
  claims: ClaimRecord[];
  chunks: ChunkRecord[];
  anomalies: Anomaly[];
};

export type CompiledView = {
  epoch: number;
  head: Head;
  messages: Message[];
  cacheBreakpoints: CacheBreakpoint[];
  manifest: Manifest;
};

export type ViewContext = {
  sessionId: string;
  turn: number;
  path: readonly StoredEntry[];
  model: ModelRef;
  prepared: PreparedCall;
  /** Session-injected head for this turn. Replay reads it back off the log. */
  head: Head;
};

export interface ViewProvider {
  view(ctx: ViewContext): Promise<CompiledView> | CompiledView;
}
