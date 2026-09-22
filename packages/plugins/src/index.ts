/**
 * Plugin surface. Plugins do not import each other.
 *
 * Contract: the context compiler is the only ViewProvider in this package.
 * Callers pass claim providers; they do not push text into the session.
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
export { rebuildFromLog } from "./compiler/rebuild.js";
