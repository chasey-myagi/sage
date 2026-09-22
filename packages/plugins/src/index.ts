/**
 * Plugin surface. Plugins do not import each other.
 *
 * Contract: the context compiler is the only ViewProvider in this package.
 * Callers pass claim providers; they do not push text into the session.
 * `replayOffline` recompiles a recorded log under another policy and does
 * not append to that log. `compareOffline` reports numbers and the offline
 * scorer note.
 *
 * Intentionally not in the Log: this barrel. Claim text reaches the Log only
 * through the manifest entry the kernel appends.
 */

export const PACKAGE = "@sage/plugins" as const;

export {
  agentsMd,
  escapeClaimText,
  progressLedger,
  systemReminder,
  toolCatalog,
} from "./claims/index.js";
export { applyDecisions } from "./compiler/apply.js";
export type { TokenCounter } from "./compiler/budget.js";
export { allocate, defaultTokenCount } from "./compiler/budget.js";
export type { Chunk } from "./compiler/chunks.js";
export { canSplitAfter, indexChunks } from "./compiler/chunks.js";
export type {
  Claim,
  ClaimProvider,
  Placement,
  Tier,
} from "./compiler/claim.js";
export { contextCompiler } from "./compiler/compiler.js";
export type { ChunkDecision } from "./compiler/eliminate.js";
export { eliminate } from "./compiler/eliminate.js";
export { expandTool } from "./compiler/expand.js";
export { rebuildFromLog } from "./compiler/rebuild.js";
export { renderStub, summarizeInput } from "./compiler/stub.js";
export type { Policy, TurnDiff } from "./replay/offline.js";
export { compareOffline, replayOffline } from "./replay/offline.js";
export {
  composePolicies,
  costTierPolicy,
  sensitivePolicy,
  withSensitiveFilter,
} from "./routing/policy.js";
export type {
  RouteInput,
  RoutePolicy,
  RouteResult,
} from "./routing/types.js";
export { modelScorer } from "./scorer/model.js";
export { ruleScorer } from "./scorer/rule.js";
export type { ScoreDecision, ScoreInput, Scorer } from "./scorer/types.js";
export { subtaskTools } from "./subtask/spawn.js";
