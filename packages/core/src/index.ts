/**
 * Kernel public surface: the Log, the View seam, the agent loop, and the
 * shared guards packages above core need.
 *
 * Guarantees: importing this module registers the built-in log kinds. This
 * package does not import plugins, tools, or apps.
 *
 * Intentionally not in the Log: the kind registry.
 */

import "./log/kinds.js";

/** Re-exported so packages that depend only on core share the one definition. */
export {
  assertNever,
  errorText,
  finiteOr,
  isFinishErrorCause,
  isFiniteNumber,
  isRecord,
  readTokenCounts,
} from "@sage/provider/contract";
export { type HealthTokens, type RunHealth, runHealth } from "./health.js";
export {
  InvalidEntryError,
  LogCorruptedError,
  UnknownEntryKindError,
} from "./log/errors.js";
export { JsonlLogStore } from "./log/jsonl.js";
export type { RequestHeaderPayload } from "./log/kinds.js";
export { MemoryLogStore } from "./log/memory.js";
export {
  epochSpan,
  isMessage,
  lastRequestHeader,
  latestViewEpoch,
  manifestsInEpoch,
  prefixMessageFor,
  requestHeaderOf,
} from "./log/path.js";
export { entryKinds, registerEntryKind } from "./log/registry.js";
export { synthesizeToolResult, TOOL_OUTCOME_UNKNOWN } from "./log/repair.js";
export type { LogStore } from "./log/store.js";
export type { StoredEntry } from "./log/types.js";
export { asEntryId } from "./log/types.js";
export { AgentSession, DEFAULT_MAX_TURNS } from "./session.js";
export type { Access, ChildRun, HarnessTool } from "./tools.js";
export {
  assertHeadFrozen,
  assertPaired,
  systemHash,
  toolsHash,
} from "./view/assert.js";
export { appendRequestEntries } from "./view/commit.js";
export {
  HeadChangedMidEpochError,
  UnpairedToolCallError,
} from "./view/errors.js";
export { passthroughView } from "./view/passthrough.js";
export {
  type ChunkRecord,
  type ClaimRecord,
  type CompiledView,
  type EntryScoredReport,
  type EpochStart,
  isTier,
  type Manifest,
  type Tier,
  type ViewContext,
  type ViewProvider,
} from "./view/types.js";
export { SAGE_DIR_NAME } from "./workspace.js";
