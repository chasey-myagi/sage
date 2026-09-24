/**
 * Jev scorer: one `choice` question per kept chunk, sent to TypeSafe
 * System One without any conversation history.
 *
 * Guarantees: the scorer never throws; every failure resolves
 * `{ decisions: [], fallback }`, so the compiler keeps the deterministic
 * tiers. Request fields, batching, and retries: `jevScorer`.
 *
 * Intentionally not in the Log: this call. The compiler copies decisions,
 * usage, and fallback onto the manifest.
 */

import { isRecord } from "@sage/core";
import {
  emptyUsage,
  type Sleeper,
  sleep,
  type Usage,
} from "@sage/provider/contract";
import { summarizeInput } from "../compiler/stub.js";
import type { ScoreDecision, Scorer } from "./types.js";

const DEFAULT_BASE_URL = "https://api.typesafe.ai";
const MODEL_ID = "jev-latest";
const DEFAULT_BATCH_SIZE = 40;
const DEFAULT_MIN_CONFIDENCE = 0.5;
const DEFAULT_TIMEOUT_MS = 10_000;
const STATE_CHAR_BUDGET = 24_000;
const CHARS_PER_TOKEN = 4;
const MAX_RETRIES = 3;
const RETRY_BASE_MS = 500;

const FULL_CRITERION = "The original text is needed.";
const NONE_CRITERION = "The one-line summary is enough.";

/** The subset of `fetch` the jev scorer calls. */
export type JevFetch = (
  url: string,
  init?: {
    method?: string;
    headers?: Record<string, string>;
    body?: string;
    signal?: AbortSignal;
  },
) => Promise<Response>;

type Listed = {
  id: string;
  tool: string;
  summary: string;
  tokens: number;
};

type Posted =
  | { ok: true; answers: Record<string, unknown>; inputTokens: number }
  | { ok: false; fallback: string };

/**
 * Creates the jev scorer. Its `name` is `jev`.
 *
 * `score` POSTs `{ state, model, questions }` to `<baseUrl>/v1/systemone`
 * (default `https://api.typesafe.ai`) with `Authorization: Bearer <apiKey>`
 * and `Content-Type: application/json`. `state` is
 * `{ query, chunks: [{ id, tool, summary, tokens }] }`; `summary` is the
 * one-line stub summary, not the original tool text. `model` is
 * `opts.model` when that string is non-empty, otherwise `jev-latest`. Each
 * chunk is one `choice` question keyed by its id, with criteria `full` and
 * `none`. `probabilities.full` >= `minConfidence` (default 0.5) becomes tier
 * `full`, otherwise `none`; the reason is `scorer:jev:<p>` with p rounded to
 * two decimals. Decisions do not set `summary`. A chunk missing from
 * `answers` is omitted, so the compiler leaves it `kept`.
 *
 * Batches hold at most `batchSize` chunks (default 40). Estimated state
 * characters are `query.length + Σ(tokens × 4)` and stay at or below 24000;
 * a chunk that alone exceeds the cap is still sent. Each batch's
 * `usage.input_tokens` sums into the returned `usage.inputOther`; the other
 * three usage fields are 0.
 *
 * Never throws. A missing `apiKey`, a non-2xx status, a timeout
 * (`timeoutMs`, default 10000), and a body that is not an answers object
 * resolve `{ decisions: [], fallback }`. 429 and 529 retry at most 3 times,
 * sleeping `500 × 2^n` milliseconds (`n` starts at 0). `prevRequest` is
 * ignored.
 */
export function jevScorer(opts: {
  apiKey?: string;
  baseUrl?: string;
  model?: string;
  /** Test seam. Production uses the global `fetch`. */
  fetchImpl?: JevFetch;
  /** Test seam. Production uses `sleep` from the provider contract. */
  sleepImpl?: Sleeper;
  batchSize?: number;
  minConfidence?: number;
  timeoutMs?: number;
}): Scorer {
  const apiKey = opts.apiKey ?? "";
  const baseUrl = (opts.baseUrl ?? DEFAULT_BASE_URL).replace(/\/+$/, "");
  const model = opts.model || MODEL_ID;
  const fetchImpl = opts.fetchImpl ?? defaultFetch;
  const sleepImpl = opts.sleepImpl ?? sleep;
  const batchSize = Math.max(
    1,
    Math.floor(opts.batchSize ?? DEFAULT_BATCH_SIZE),
  );
  const minConfidence = opts.minConfidence ?? DEFAULT_MIN_CONFIDENCE;
  const timeoutMs = opts.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const url = `${baseUrl}/v1/systemone`;

  return {
    name: "jev",
    async score(input) {
      try {
        if (!apiKey) {
          return { decisions: [], fallback: "no-api-key", requested: false };
        }
        if (input.candidates.length === 0) return { decisions: [] };
        const batches = packBatches(input.query, input.candidates, batchSize);
        const decisions: ScoreDecision[] = [];
        let inputOther = 0;
        for (const batch of batches) {
          const posted = await postBatch({
            fetchImpl,
            sleepImpl,
            url,
            apiKey,
            timeoutMs,
            signal: input.signal,
            body: JSON.stringify({
              state: { query: input.query, chunks: batch },
              model,
              questions: questionsFor(batch),
            }),
          });
          if (!posted.ok) return { decisions: [], fallback: posted.fallback };
          inputOther += posted.inputTokens;
          decisions.push(...decisionsFor(batch, posted.answers, minConfidence));
        }
        return { decisions, usage: usageFrom(inputOther) };
      } catch {
        return { decisions: [], fallback: "error" };
      }
    },
  };
}

function defaultFetch(
  url: string,
  init?: {
    method?: string;
    headers?: Record<string, string>;
    body?: string;
    signal?: AbortSignal;
  },
): Promise<Response> {
  return globalThis.fetch(url, init);
}

function packBatches(
  query: string,
  chunks: readonly {
    id: string;
    toolName?: string;
    input?: unknown;
    tokens: number;
  }[],
  batchSize: number,
): Listed[][] {
  const batches: Listed[][] = [];
  let current: Listed[] = [];
  for (const chunk of chunks) {
    const item = listChunk(chunk);
    const overCount = current.length >= batchSize;
    const overChars =
      current.length > 0 &&
      estimatedChars(query, current) + item.tokens * CHARS_PER_TOKEN >
        STATE_CHAR_BUDGET;
    if (overCount || overChars) {
      batches.push(current);
      current = [];
    }
    current.push(item);
  }
  if (current.length > 0) batches.push(current);
  return batches;
}

function listChunk(chunk: {
  id: string;
  toolName?: string;
  input?: unknown;
  tokens: number;
}): Listed {
  return {
    id: chunk.id,
    tool: chunk.toolName ?? "",
    summary: summarizeInput(chunk.input),
    tokens: chunk.tokens,
  };
}

function estimatedChars(query: string, items: readonly Listed[]): number {
  let chars = query.length;
  for (const item of items) chars += item.tokens * CHARS_PER_TOKEN;
  return chars;
}

function questionsFor(batch: readonly Listed[]): Record<string, unknown> {
  const questions: Record<string, unknown> = {};
  for (const item of batch) {
    questions[item.id] = {
      type: "choice",
      instructions: `To finish the task in state.query, will the next step need the full original text of chunk ${item.id}? Answer full if the original text is needed, none if the one-line summary is enough.`,
      criteria: {
        full: FULL_CRITERION,
        none: NONE_CRITERION,
      },
    };
  }
  return questions;
}

function decisionsFor(
  batch: readonly Listed[],
  answers: Record<string, unknown>,
  minConfidence: number,
): ScoreDecision[] {
  const decisions: ScoreDecision[] = [];
  for (const item of batch) {
    const full = readFull(answers[item.id]);
    if (full === undefined) continue;
    decisions.push({
      id: item.id,
      tier: full >= minConfidence ? "full" : "none",
      reason: `scorer:jev:${full.toFixed(2)}`,
    });
  }
  return decisions;
}

function readFull(answer: unknown): number | undefined {
  if (!isRecord(answer) || !isRecord(answer.probabilities)) return undefined;
  const full = answer.probabilities.full;
  if (typeof full !== "number" || !Number.isFinite(full)) return undefined;
  return full;
}

async function postBatch(input: {
  fetchImpl: JevFetch;
  sleepImpl: Sleeper;
  url: string;
  apiKey: string;
  timeoutMs: number;
  signal: AbortSignal | undefined;
  body: string;
}): Promise<Posted> {
  for (let attempt = 0; attempt <= MAX_RETRIES; attempt += 1) {
    const result = await once(input);
    if (result.kind === "fallback")
      return { ok: false, fallback: result.fallback };
    if (result.status === 429 || result.status === 529) {
      if (attempt === MAX_RETRIES) {
        return { ok: false, fallback: `http:${result.status}` };
      }
      await input.sleepImpl(RETRY_BASE_MS * 2 ** attempt);
      continue;
    }
    if (result.status < 200 || result.status >= 300) {
      return { ok: false, fallback: `http:${result.status}` };
    }
    const parsed = parseBody(result.text);
    if (!parsed) return { ok: false, fallback: "invalid-json" };
    return { ok: true, ...parsed };
  }
  return { ok: false, fallback: "error" };
}

async function once(input: {
  fetchImpl: JevFetch;
  url: string;
  apiKey: string;
  timeoutMs: number;
  signal: AbortSignal | undefined;
  body: string;
}): Promise<
  | { kind: "http"; status: number; text: string }
  | { kind: "fallback"; fallback: string }
> {
  const controller = new AbortController();
  let cause: "timeout" | "aborted" | undefined;
  const timer = setTimeout(() => {
    cause = cause ?? "timeout";
    controller.abort();
  }, input.timeoutMs);
  const onCallerAbort = () => {
    cause = cause ?? "aborted";
    controller.abort();
  };
  if (input.signal?.aborted) onCallerAbort();
  else input.signal?.addEventListener("abort", onCallerAbort, { once: true });
  try {
    const response = await input.fetchImpl(input.url, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${input.apiKey}`,
        "Content-Type": "application/json",
      },
      body: input.body,
      signal: controller.signal,
    });
    if (cause === "timeout") return { kind: "fallback", fallback: "timeout" };
    if (cause === "aborted") return { kind: "fallback", fallback: "aborted" };
    return {
      kind: "http",
      status: response.status,
      text: await response.text(),
    };
  } catch {
    if (cause === "timeout") return { kind: "fallback", fallback: "timeout" };
    if (cause === "aborted") return { kind: "fallback", fallback: "aborted" };
    return { kind: "fallback", fallback: "error" };
  } finally {
    clearTimeout(timer);
    input.signal?.removeEventListener("abort", onCallerAbort);
  }
}

function parseBody(
  text: string,
): { answers: Record<string, unknown>; inputTokens: number } | undefined {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return undefined;
  }
  if (!isRecord(parsed) || !isRecord(parsed.answers)) return undefined;
  return { answers: parsed.answers, inputTokens: readInputTokens(parsed) };
}

function readInputTokens(body: Record<string, unknown>): number {
  if (!isRecord(body.usage)) return 0;
  const tokens = body.usage.input_tokens;
  if (typeof tokens !== "number" || !Number.isFinite(tokens)) return 0;
  return tokens;
}

function usageFrom(inputOther: number): Usage {
  return { ...emptyUsage(), inputOther };
}
