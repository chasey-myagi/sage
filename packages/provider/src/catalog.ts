/**
 * Static model table: context window and pricing per known model, plus the
 * `adapterDefaults` an adapter reports for a call.
 *
 * Guarantees: lookup is keyed by (provider, model) over the built-in table
 * plus the caller's local entries, so one model id may carry different
 * prices on different providers. An unknown model still prepares, with
 * fallback numbers, `pricing: null`, and `adapterDefaults.catalogFallback:
 * true`. Built-in prices were checked on 2026-09-23 against each entry's
 * `source`. `currency` absent means USD; cache read/write ratios divide the
 * four prices and ignore currency. `cacheWritePerMTok` is what writing a
 * prefix into the cache costs: providers whose page bills a cache miss at
 * the input price with no creation fee list the input price there.
 *
 * Intentionally not in the Log: the table itself, built-in or local. Lookup
 * happens before the call; the session stores the returned pricing, window,
 * and `adapterDefaults` on request_header, and replay reads those.
 */

import type { PreparedCall, Pricing } from "./contract/adapter.js";
import type { ModelRef, Request } from "./contract/message.js";
import type { ClientOptions } from "./http.js";

/** Context window, in tokens, assumed for a model missing from the table. */
const FALLBACK_WINDOW = 128_000;
/** Output token cap (`max_tokens`) for a model missing from the table. */
const FALLBACK_MAX_TOKENS = 4096;
/**
 * Output token cap (`max_tokens` on the wire) reported as
 * `adapterDefaults.maxTokens` for every model in the table. It limits one
 * response's output and is unrelated to the context window, which is why a
 * 1M-context model shares it. `Sampling.maxTokens` overrides it.
 */
const KNOWN_MAX_TOKENS = 8192;
const ANTHROPIC_SOURCE =
  "https://platform.claude.com/docs/en/about-claude/pricing";
const OPENAI_SOURCE = "https://developers.openai.com/api/docs/pricing";
const DEEPSEEK_SOURCE = "https://api-docs.deepseek.com/quick_start/pricing";
const MOONSHOT_SOURCE = "https://platform.kimi.com/docs/pricing/chat";
const DASHSCOPE_SOURCE =
  "https://help.aliyun.com/zh/model-studio/model-pricing";
const TYPESAFE_SOURCE =
  "https://typesafe.ai/blog/introducing-system-one-models-and-jev";

/**
 * Adapter constructor options: the transport options plus `models`,
 * catalog entries beyond the built-in table (usually the user's providers
 * file after `parseLocalProviders`). `prepare` looks models up by
 * (provider, model) in both. Omitted `models` means built-ins only.
 */
export type AdapterOptions = ClientOptions & {
  models?: readonly CatalogEntry[];
};

/** One known model. `contextWindow` is in tokens; prices are per million tokens. */
export type CatalogEntry = {
  provider: string;
  model: string;
  contextWindow: number;
  pricing: Pricing;
};

/** The built-in table. */
export const MODELS: readonly CatalogEntry[] = [
  {
    provider: "anthropic",
    model: "claude-sonnet-5",
    contextWindow: 1_000_000,
    pricing: {
      inputPerMTok: 2,
      outputPerMTok: 10,
      cacheWritePerMTok: 2.5,
      cacheReadPerMTok: 0.2,
      source: ANTHROPIC_SOURCE,
    },
  },
  {
    provider: "anthropic",
    model: "claude-opus-5",
    contextWindow: 1_000_000,
    pricing: {
      inputPerMTok: 5,
      outputPerMTok: 25,
      cacheWritePerMTok: 6.25,
      cacheReadPerMTok: 0.5,
      source: ANTHROPIC_SOURCE,
    },
  },
  {
    provider: "anthropic",
    model: "claude-haiku-4-5-20251001",
    contextWindow: 200_000,
    pricing: {
      inputPerMTok: 1,
      outputPerMTok: 5,
      cacheWritePerMTok: 1.25,
      cacheReadPerMTok: 0.1,
      source: ANTHROPIC_SOURCE,
    },
  },
  {
    provider: "anthropic",
    model: "claude-fable-5-1",
    contextWindow: 1_000_000,
    pricing: {
      inputPerMTok: 10,
      outputPerMTok: 50,
      cacheWritePerMTok: 12.5,
      cacheReadPerMTok: 0.25,
      source: ANTHROPIC_SOURCE,
    },
  },
  {
    provider: "openai",
    model: "gpt-5.4",
    contextWindow: 272_000,
    pricing: {
      inputPerMTok: 2.5,
      outputPerMTok: 15,
      cacheWritePerMTok: 2.5,
      cacheReadPerMTok: 0.25,
      source: OPENAI_SOURCE,
    },
  },
  {
    provider: "openai",
    model: "gpt-5.5",
    contextWindow: 272_000,
    pricing: {
      inputPerMTok: 5,
      outputPerMTok: 30,
      cacheWritePerMTok: 5,
      cacheReadPerMTok: 0.5,
      source: OPENAI_SOURCE,
    },
  },
  {
    provider: "openai",
    model: "gpt-5.4-mini",
    contextWindow: 272_000,
    pricing: {
      inputPerMTok: 0.75,
      outputPerMTok: 4.5,
      cacheWritePerMTok: 0.75,
      cacheReadPerMTok: 0.075,
      source: OPENAI_SOURCE,
    },
  },
  // Peak-hour prices. Off-peak is half price; peak is 01:00-04:00 and
  // 06:00-10:00 UTC on weekdays.
  {
    provider: "deepseek",
    model: "deepseek-flash",
    contextWindow: 1_000_000,
    pricing: {
      inputPerMTok: 0.3,
      outputPerMTok: 1.2,
      cacheWritePerMTok: 0.3,
      cacheReadPerMTok: 0.006,
      source: DEEPSEEK_SOURCE,
    },
  },
  {
    provider: "deepseek",
    model: "deepseek-v4-pro",
    contextWindow: 1_000_000,
    pricing: {
      inputPerMTok: 1.32,
      outputPerMTok: 3.96,
      cacheWritePerMTok: 1.32,
      cacheReadPerMTok: 0.044,
      source: DEEPSEEK_SOURCE,
    },
  },
  {
    provider: "moonshot",
    model: "kimi-k2.6",
    contextWindow: 262_144,
    pricing: {
      inputPerMTok: 6.5,
      outputPerMTok: 27,
      cacheWritePerMTok: 6.5,
      cacheReadPerMTok: 1.1,
      source: MOONSHOT_SOURCE,
      currency: "CNY",
    },
  },
  {
    provider: "moonshot",
    model: "kimi-k2.7-code",
    contextWindow: 262_144,
    pricing: {
      inputPerMTok: 6.5,
      outputPerMTok: 27,
      cacheWritePerMTok: 6.5,
      cacheReadPerMTok: 1.3,
      source: MOONSHOT_SOURCE,
      currency: "CNY",
    },
  },
  // Explicit context cache prices at the 5-minute TTL.
  {
    provider: "moonshot",
    model: "kimi-k3",
    contextWindow: 1_048_576,
    pricing: {
      inputPerMTok: 20,
      outputPerMTok: 100,
      cacheWritePerMTok: 20,
      cacheReadPerMTok: 2,
      source: MOONSHOT_SOURCE,
      currency: "CNY",
    },
  },
  // List prices; promotions may be lower. Thinking-mode output costs more
  // (8 for plus). Context: plus and flash 1,000,000; max 32,768. flash uses
  // the first tier, for inputs up to 128K.
  {
    provider: "dashscope",
    model: "qwen-plus",
    contextWindow: 1_000_000,
    pricing: {
      inputPerMTok: 0.8,
      outputPerMTok: 2,
      cacheWritePerMTok: 0.8,
      cacheReadPerMTok: 0.08,
      source: DASHSCOPE_SOURCE,
      currency: "CNY",
    },
  },
  {
    provider: "dashscope",
    model: "qwen-max",
    contextWindow: 32_768,
    pricing: {
      inputPerMTok: 2.4,
      outputPerMTok: 9.6,
      cacheWritePerMTok: 2.4,
      cacheReadPerMTok: 0.24,
      source: DASHSCOPE_SOURCE,
      currency: "CNY",
    },
  },
  {
    provider: "dashscope",
    model: "qwen3.8-max",
    contextWindow: 32_768,
    pricing: {
      inputPerMTok: 12,
      outputPerMTok: 36,
      cacheWritePerMTok: 12,
      cacheReadPerMTok: 1.2,
      source: DASHSCOPE_SOURCE,
      currency: "CNY",
    },
  },
  {
    provider: "dashscope",
    model: "qwen-flash",
    contextWindow: 1_000_000,
    pricing: {
      inputPerMTok: 0.15,
      outputPerMTok: 1.5,
      cacheWritePerMTok: 0.15,
      cacheReadPerMTok: 0.015,
      source: DASHSCOPE_SOURCE,
      currency: "CNY",
    },
  },
  {
    provider: "typesafe",
    model: "jev-latest",
    contextWindow: 32_000,
    pricing: {
      inputPerMTok: 0.042,
      outputPerMTok: 0,
      cacheWritePerMTok: 0,
      cacheReadPerMTok: 0,
      source: TYPESAFE_SOURCE,
    },
  },
];

/**
 * Known models for `provider`: built-in entries in table order, then
 * `local` entries in file order; empty when none.
 */
export function listModels(
  provider: string,
  local: readonly CatalogEntry[] = [],
): readonly CatalogEntry[] {
  return [...MODELS, ...local].filter((entry) => entry.provider === provider);
}

/**
 * The first {@link listModels} entry for `provider`, or undefined when the
 * provider has none (a user provider whose models are not declared).
 */
export function defaultModel(
  provider: string,
  local: readonly CatalogEntry[] = [],
): ModelRef | undefined {
  const first = listModels(provider, local)[0];
  return first ? { provider: first.provider, model: first.model } : undefined;
}

/**
 * The prepared call for `model`: the (provider, model) entry from the
 * built-in table or `local` when known, fallback values with
 * `catalogFallback: true` otherwise. Never throws.
 */
export function prepareModel(
  model: ModelRef,
  local: readonly CatalogEntry[] = [],
): PreparedCall {
  const hit = listModels(model.provider, local).find(
    (entry) => entry.model === model.model,
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
    pricing: hit.pricing,
    adapterDefaults: { maxTokens: KNOWN_MAX_TOKENS },
  };
}

/**
 * `max_tokens` for the wire: `request.sampling.maxTokens` when set, else
 * `adapterDefaults.maxTokens`, else `FALLBACK_MAX_TOKENS`.
 */
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
