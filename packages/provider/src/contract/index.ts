/**
 * Contract barrel. Types, plus the pure `collect` fold.
 *
 * Deliberately not a Log entry: nothing in this module performs I/O.
 */

export type {
  AssistantMessage,
  PreparedCall,
  Pricing,
  ProviderAdapter,
} from "./adapter.js";
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
export type { FinishError, FinishKind, StreamChunk } from "./stream.js";
export type { Usage } from "./usage.js";
export type { ContractVersion } from "./version.js";
