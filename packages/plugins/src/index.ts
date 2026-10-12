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
export { type Rates, ratesFor } from "./compiler/decide.js";
export { EXPAND_TOOL_NAME, expandTool } from "./compiler/expand.js";
export {
  type EntryJevRequest,
  type EntryJevScan,
  entryJevRequests,
} from "./replay/jev-entry.js";
export {
  costTierPolicy,
  sensitivePolicy,
  withSensitiveFilter,
} from "./routing/policy.js";
export type { RoutePolicy } from "./routing/types.js";
export { type BashKind, classifyBashCommand } from "./scorer/bash-kind.js";
export {
  bashSearchPatterns,
  normalizeBashCommand,
} from "./scorer/bash-search.js";
export {
  type JevEndpoint,
  type Posted,
  type PostHooks,
  postMany,
} from "./scorer/jev/http.js";
export {
  type Judgment,
  readJudgment,
  type TriageJudgment,
} from "./scorer/jev/questions.js";
export {
  type ChunkRequest,
  estimateRequestTokens,
  requestBody,
} from "./scorer/jev/state.js";
export {
  DEFAULT_JEV_THRESHOLDS,
  decideTier,
  type JevThresholds,
} from "./scorer/jev/tiers.js";
export { JEV_MODEL_ID, jevScorer } from "./scorer/jev.js";
export { modelScorer } from "./scorer/model.js";
export {
  DEFAULT_RULE_TIERS,
  type RuleCategory,
  type RuleTier,
  ruleCategory,
  ruleScorer,
} from "./scorer/rule.js";
export type { Scorer } from "./scorer/types.js";
export { subtaskTools } from "./subtask/spawn.js";
