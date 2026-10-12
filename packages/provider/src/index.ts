/**
 * Provider package entry: the two adapters, the model catalog, endpoint
 * presets, and the contract types callers name alongside them. Callers
 * above core use this; core uses `@sage/provider/contract` only.
 *
 * Intentionally not in the Log: this module only re-exports. API keys,
 * wire bytes, and abort signals stay inside the adapters.
 */

export { AnthropicAdapter } from "./anthropic/index.js";
export type { CatalogEntry } from "./catalog.js";
export { defaultModel, listModels } from "./catalog.js";
export type { ModelRef, ProviderAdapter, Sampling } from "./contract/index.js";
export { OpenAIAdapter } from "./openai/index.js";
export { PRESETS, resolvePreset } from "./presets.js";
