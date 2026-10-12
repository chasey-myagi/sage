/**
 * Plugin surface. Plugins do not import each other.
 *
 * Contract: the context compiler is the only ViewProvider in this package.
 * Callers pass claim providers; they do not push text into the session.
 *
 * Intentionally not in the Log: this barrel. Claim text reaches the Log only
 * through the manifest entry the kernel appends.
 */

export {
  agentsMd,
  progressLedger,
  systemReminder,
  toolCatalog,
} from "./claims/index.js";
export { toolUseIdOfChunk } from "./compiler/chunks.js";
export type { ClaimProvider } from "./compiler/claim.js";
export {
  contextCompiler,
  DEFAULT_COMPILER_BUDGET,
} from "./compiler/compiler.js";
export { EXPAND_TOOL_NAME, expandTool } from "./compiler/expand.js";
export {
  costTierPolicy,
  sensitivePolicy,
  withSensitiveFilter,
} from "./routing/policy.js";
export type { RoutePolicy } from "./routing/types.js";
export { jevScorer } from "./scorer/jev.js";
export { modelScorer } from "./scorer/model.js";
export { ruleScorer } from "./scorer/rule.js";
export type { Scorer } from "./scorer/types.js";
export { subtaskTools } from "./subtask/spawn.js";
