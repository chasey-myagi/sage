/**
 * Adapter surface. Adapters translate; they do not choose cache breakpoints.
 *
 * Deliberately not a Log entry: `adapterDefaults` is reported so a later
 * request_header entry can store it (#4). Pricing stays null until a source
 * URL is checked (docs/design/references.md §3).
 */

import type { ContentBlock, ModelRef, Request } from "./message.js";
import type { StreamChunk } from "./stream.js";
import type { Usage } from "./usage.js";

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

export type PreparedCall = {
  model: ModelRef;
  contextWindow: number;
  pricing: Pricing | null;
  adapterDefaults: Record<string, unknown>;
};

export interface ProviderAdapter {
  readonly api: "anthropic-messages" | "openai-chat";
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
 * responseModel and responseId are what the provider sent in the response
 * chunk, not the requested model. Both are "" when no response chunk came.
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
