/**
 * Static model table.
 *
 * Pricing is unverified — see docs/design/references.md §3.
 * When a price is filled in, it must carry a source URL.
 *
 * Deliberately not a Log entry: lookup happens before the call.
 * adapterDefaults are reported so a later request_header can store them.
 */

import type { PreparedCall } from "./contract/adapter.js";
import type { ModelRef, Request } from "./contract/message.js";

const FALLBACK_WINDOW = 128_000;
const FALLBACK_MAX_TOKENS = 4096;
const KNOWN_MAX_TOKENS = 8192;

export type CatalogEntry = {
  provider: string;
  model: string;
  contextWindow: number;
};

const MODELS: readonly CatalogEntry[] = [
  {
    provider: "anthropic",
    model: "claude-sonnet-4-5",
    contextWindow: 200_000,
  },
  { provider: "anthropic", model: "claude-opus-4-1", contextWindow: 200_000 },
  {
    provider: "anthropic",
    model: "claude-haiku-4-5",
    contextWindow: 200_000,
  },
  { provider: "openai", model: "gpt-5", contextWindow: 400_000 },
  { provider: "openai", model: "gpt-5-mini", contextWindow: 400_000 },
];

export function listModels(provider: string): readonly CatalogEntry[] {
  return MODELS.filter((entry) => entry.provider === provider);
}

export function defaultModel(provider: string): ModelRef {
  const first = listModels(provider)[0];
  if (!first) {
    throw new Error(`No models for provider: ${provider}`);
  }
  return { provider: first.provider, model: first.model };
}

export function prepareModel(model: ModelRef): PreparedCall {
  const hit = MODELS.find(
    (entry) => entry.provider === model.provider && entry.model === model.model,
  );
  if (!hit) {
    return {
      model,
      contextWindow: FALLBACK_WINDOW,
      pricing: null,
      adapterDefaults: {
        maxTokens: FALLBACK_MAX_TOKENS,
        catalogFallback: true,
      },
    };
  }
  return {
    model,
    contextWindow: hit.contextWindow,
    pricing: null,
    adapterDefaults: { maxTokens: KNOWN_MAX_TOKENS },
  };
}

export function resolveMaxTokens(
  request: Request,
  prepared: PreparedCall,
): number {
  if (typeof request.sampling.maxTokens === "number") {
    return request.sampling.maxTokens;
  }
  const value = prepared.adapterDefaults.maxTokens;
  return typeof value === "number" ? value : FALLBACK_MAX_TOKENS;
}
