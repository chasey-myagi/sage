/**
 * Contract barrel. Types, the pure `collect` fold, and the dependency-free
 * helpers every layer above shares (`isRecord`, `isFiniteNumber`,
 * `errorText`, `ModelRef` readers, usage arithmetic, backoff).
 *
 * Guarantees: nothing under this path performs I/O or imports anything
 * outside it; the only side effect is the timer inside `sleep`. core may
 * import only this subpath of the provider package.
 *
 * Intentionally not in the Log: nothing here appends.
 */

export {
  type AssistantMessage,
  isPricing,
  type PreparedCall,
  type Pricing,
  type ProviderAdapter,
} from "./adapter.js";
export { backoffMs, type Sleeper, sleep } from "./backoff.js";
export { collect } from "./collect.js";
export { errorText } from "./error.js";
export type {
  CacheBreakpoint,
  ContentBlock,
  Head,
  Message,
  ModelRef,
  Request,
  Role,
  Sampling,
  TextBlock,
  ThinkingBlock,
  ToolChoice,
  ToolDef,
  ToolResultBlock,
  ToolUseBlock,
} from "./message.js";
export { isModelRef, parseModelRef } from "./model-ref.js";
export { finiteOr, isFiniteNumber, isRecord } from "./record.js";
export type { FinishKind, StreamChunk } from "./stream.js";
export {
  addUsage,
  emptyUsage,
  inputTokens,
  readTokenCounts,
  type TokenCounts,
  totalTokens,
  type Usage,
} from "./usage.js";
