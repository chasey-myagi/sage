/**
 * Contract barrel. Types, the pure `collect` fold, and the dependency-free
 * helpers every layer above shares (`isRecord`, usage arithmetic, backoff).
 *
 * Deliberately not a Log entry: nothing in this module performs I/O. The
 * only side effect anywhere in it is the timer inside `sleep`.
 */

export type {
  AssistantMessage,
  PreparedCall,
  Pricing,
  ProviderAdapter,
} from "./adapter.js";
export { backoffMs, type Sleeper, sleep } from "./backoff.js";
export { collect } from "./collect.js";
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
export { isRecord } from "./record.js";
export type { FinishError, FinishKind, StreamChunk } from "./stream.js";
export {
  addUsage,
  emptyUsage,
  inputTokens,
  type TokenCounts,
  totalTokens,
  type Usage,
} from "./usage.js";
export type { ContractVersion } from "./version.js";
