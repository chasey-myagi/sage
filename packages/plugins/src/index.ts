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

export type { TokenCounter } from "./compiler/budget.js";
export { allocate, defaultTokenCount } from "./compiler/budget.js";
export type {
  Claim,
  ClaimProvider,
  Placement,
  Tier,
} from "./compiler/claim.js";
export { contextCompiler } from "./compiler/compiler.js";
export { rebuildFromLog } from "./compiler/rebuild.js";
