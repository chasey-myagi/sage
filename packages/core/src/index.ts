/**
 * Kernel public surface.
 *
 * Contract: importing this module registers the built-in log kinds. Callers
 * see Log and the View seam. This package does not import plugins.
 *
 * Intentionally not in the Log: package identity and the kind registry.
 */

import "./log/kinds.js";

export const PACKAGE = "@sage/core" as const;

export { type RunHealth, runHealth } from "./health.js";
export type { Decision, HookEvents, Hooks } from "./hooks/types.js";
export {
  InvalidEntryError,
  LogCorruptedError,
  UnknownEntryKindError,
} from "./log/errors.js";
export { JsonlLogStore, type LogFs } from "./log/jsonl.js";
export { MemoryLogStore } from "./log/memory.js";
export {
  type EntryKindDef,
  entryKinds,
  registerEntryKind,
} from "./log/registry.js";
export {
  DANGLING,
  synthesizeToolResult,
  type ToolResultBlock,
} from "./log/repair.js";
export { type LogStore, replay } from "./log/store.js";
export type {
  EntryId,
  EntryInput,
  ModelRef,
  StoredEntry,
} from "./log/types.js";
export { AgentSession, type RunSummary, type TurnOutcome } from "./session.js";
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
  ViewContext,
  ViewProvider,
} from "./view/types.js";
