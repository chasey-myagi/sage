/**
 * Provider package entry.
 *
 * Deliberately not a Log entry: this module only re-exports. API keys,
 * wire bytes, and abort signals stay inside the adapters.
 */

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
  ToolDef,
  ToolResultBlock,
  ToolUseBlock,
  Usage,
} from "./contract/index.js";
export { normalizeForApi } from "./normalize.js";
export { OpenAIAdapter } from "./openai/index.js";
