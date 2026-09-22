/**
 * Static model table.
 *
 * 定价 2026-09-23 核对，来源见每条 source。
 * currency 缺省为 USD。缓存读写比率只除四个单价，不看币种。
 *
 * Deliberately not a Log entry: lookup happens before the call.
 * adapterDefaults are reported so a later request_header can store them.
 */

import type { PreparedCall, Pricing } from "./contract/adapter.js";
import type { ModelRef, Request } from "./contract/message.js";

const FALLBACK_WINDOW = 128_000;
const FALLBACK_MAX_TOKENS = 4096;
const KNOWN_MAX_TOKENS = 8192;
const ANTHROPIC_SOURCE =
  "https://platform.claude.com/docs/en/about-claude/pricing";
const OPENAI_SOURCE = "https://developers.openai.com/api/docs/pricing";
const DEEPSEEK_SOURCE = "https://api-docs.deepseek.com/quick_start/pricing";
const MOONSHOT_SOURCE = "https://platform.kimi.com/docs/pricing/chat";
const DASHSCOPE_SOURCE =
  "https://help.aliyun.com/zh/model-studio/model-pricing";

export type CatalogEntry = {
  provider: string;
  model: string;
  contextWindow: number;
  pricing: Pricing;
};

const MODELS: readonly CatalogEntry[] = [
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
      cacheWritePerMTok: 0,
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
      cacheWritePerMTok: 0,
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
      cacheWritePerMTok: 0,
      cacheReadPerMTok: 0.075,
      source: OPENAI_SOURCE,
    },
  },
  // 非峰时减半：01:00–04:00 与 06:00–10:00 UTC 工作日为峰时
  {
    provider: "deepseek",
    model: "deepseek-flash",
    contextWindow: 1_000_000,
    pricing: {
      inputPerMTok: 0.3,
      outputPerMTok: 1.2,
      cacheWritePerMTok: 0,
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
      cacheWritePerMTok: 0,
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
      cacheWritePerMTok: 0,
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
      cacheWritePerMTok: 0,
      cacheReadPerMTok: 1.3,
      source: MOONSHOT_SOURCE,
      currency: "CNY",
    },
  },
  // 显式上下文缓存 5min TTL 价
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
  // 页面为原价，促销可能更低；思考模式输出价不同（plus 为 8）。
  // plus / flash 上下文 1,000,000；max 为 32,768。
  // flash 单价是输入 ≤128K 的第一档。
  {
    provider: "dashscope",
    model: "qwen-plus",
    contextWindow: 1_000_000,
    pricing: {
      inputPerMTok: 0.8,
      outputPerMTok: 2,
      cacheWritePerMTok: 0,
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
      inputPerMTok: 12,
      outputPerMTok: 36,
      cacheWritePerMTok: 0,
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
      cacheWritePerMTok: 0,
      cacheReadPerMTok: 0.015,
      source: DASHSCOPE_SOURCE,
      currency: "CNY",
    },
  },
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
    pricing: hit.pricing,
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
