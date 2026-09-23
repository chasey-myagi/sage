/**
 * Kernel public surface: the Log, the View seam, the agent loop, and the
 * shared guards packages above core need.
 *
 * Guarantees: importing this module registers the built-in log kinds. This
 * package does not import plugins, tools, or apps.
 *
 * Intentionally not in the Log: package identity and the kind registry.
 */

import "./log/kinds.js";

/** Package name, for diagnostics. */
export const PACKAGE = "@sage/core" as const;

/** Re-exported so packages that depend only on core share the one guard. */
export { isRecord } from "@sage/provider/contract";
export { type HealthTokens, type RunHealth, runHealth } from "./health.js";
export type { Decision, HookEvents, Hooks } from "./hooks/types.js";
export {
  InvalidEntryError,
  LogCorruptedError,
  UnknownEntryKindError,
} from "./log/errors.js";
export { JsonlLogStore, type LogFs } from "./log/jsonl.js";
export { MemoryLogStore } from "./log/memory.js";
export { latestViewEpoch } from "./log/path.js";
export {
  type EntryKindDef,
  entryKinds,
  registerEntryKind,
} from "./log/registry.js";
export { DANGLING, synthesizeToolResult } from "./log/repair.js";
export { type LogStore, replay } from "./log/store.js";
export type {
  EntryId,
  EntryInput,
  ModelRef,
  StoredEntry,
} from "./log/types.js";
export {
  AgentSession,
  DEFAULT_MAX_TURNS,
  type RunSummary,
  type TurnOutcome,
} from "./session.js";
export type { Access, HarnessTool } from "./tools.js";
export {
  assertHeadFrozen,
  assertPaired,
  headHash,
  systemHash,
  toolsHash,
} from "./view/assert.js";
export {
  HeadChangedMidEpochError,
  UnpairedToolCallError,
} from "./view/errors.js";
export { passthroughView } from "./view/passthrough.js";
export type {
  Anomaly,
  ChunkRecord,
  ClaimRecord,
  CompiledView,
  EntryScoredReport,
  Manifest,
  Tier,
  TierRecord,
  ViewContext,
  ViewProvider,
} from "./view/types.js";
