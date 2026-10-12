/**
 * The adapter interface every provider implements. Adapters translate
 * requests and streams; they do not choose cache breakpoints.
 *
 * Guarantees: `stream` never throws; failures end as a finish chunk.
 * Pricing stays null until a source URL is checked
 * (docs/design/references.md §3).
 *
 * Intentionally not in the Log: the adapter object. `adapterDefaults` is
 * reported so the session can store it on the request_header entry.
 */

import type { ContentBlock, ModelRef, Request } from "./message.js";
import { isFiniteNumber, isRecord } from "./record.js";
import type { StreamChunk } from "./stream.js";
import type { Usage } from "./usage.js";

/** Prices per million tokens, with the page they were checked against. */
export type Pricing = {
  inputPerMTok: number;
  outputPerMTok: number;
  cacheReadPerMTok: number;
  cacheWritePerMTok: number;
  source: string;
  /**
   * Billing currency of the per-MTok prices. Absent means USD.
   * Cache rate ratios use only the four prices, never this field.
   */
  currency?: "USD" | "CNY";
};

/**
 * Narrows untrusted JSON to `Pricing`: four finite prices, a string
 * `source`, and `currency` absent or `USD` / `CNY`. Other fields are
 * ignored.
 */
export function isPricing(value: unknown): value is Pricing {
  if (!isRecord(value)) return false;
  const prices = [
    value.inputPerMTok,
    value.outputPerMTok,
    value.cacheReadPerMTok,
    value.cacheWritePerMTok,
  ];
  if (!prices.every(isFiniteNumber)) return false;
  if (typeof value.source !== "string") return false;
  return (
    value.currency === undefined ||
    value.currency === "USD" ||
    value.currency === "CNY"
  );
}

/**
 * What an adapter knows about a model before calling it. `contextWindow` is
 * in tokens. `adapterDefaults` are the values the adapter fills in when the
 * request leaves them unset (for example `maxTokens`).
 */
export type PreparedCall = {
  model: ModelRef;
  contextWindow: number;
  pricing: Pricing | null;
  adapterDefaults: Record<string, unknown>;
};

/** One provider protocol. Adapters are long-lived and hold no per-call state. */
export interface ProviderAdapter {
  readonly api: "anthropic-messages" | "openai-chat";
  /** Catalog lookup for `model`. Never throws; unknown models get fallbacks. */
  prepare(model: ModelRef): PreparedCall;
  /**
   * Does not throw. Transport and protocol failures end as
   * finish{kind:"error"}. Abort ends as finish{kind:"aborted"}.
   * Fetch throws, HTTP 429, and HTTP 5xx are retried before the
   * response body starts. A started body is not retried.
   */
  stream(
    request: Request,
    prepared: PreparedCall,
    signal?: AbortSignal,
  ): AsyncIterable<StreamChunk>;
}

/**
 * One model call folded into a message by `collect`. responseModel and
 * responseId are what the provider sent in the response chunk, not the
 * requested model. Both are "" when no response chunk came.
 */
export type AssistantMessage = {
  role: "assistant";
  content: ContentBlock[];
  usage: Usage;
  finish: Extract<StreamChunk, { type: "finish" }>;
  responseModel: string;
  responseId: string;
  api: ProviderAdapter["api"];
};
