/**
 * Provider package entry: the two adapters, the model catalog, endpoint
 * presets, and the contract types. Callers above core use this; core uses
 * `@sage/provider/contract` only.
 *
 * Intentionally not in the Log: this module only re-exports. API keys,
 * wire bytes, and abort signals stay inside the adapters.
 */

/** Package name, for diagnostics. */
export const PACKAGE = "@sage/provider" as const;

export { AnthropicAdapter } from "./anthropic/index.js";
export type { CatalogEntry } from "./catalog.js";
export { defaultModel, listModels } from "./catalog.js";
export { collect } from "./contract/collect.js";
export type {
  AssistantMessage,
  CacheBreakpoint,
  ContentBlock,
  ContractVersion,
  FinishKind,
  Head,
  Message,
  ModelRef,
  PreparedCall,
  Pricing,
  ProviderAdapter,
  Request,
  Role,
  Sampling,
  StreamChunk,
  TextBlock,
  ThinkingBlock,
  ToolChoice,
  ToolDef,
  ToolResultBlock,
  ToolUseBlock,
  Usage,
} from "./contract/index.js";
export { normalizeForApi } from "./normalize.js";
export { OpenAIAdapter } from "./openai/index.js";
export type { ProviderPreset } from "./presets.js";
export { PRESETS, resolvePreset } from "./presets.js";
