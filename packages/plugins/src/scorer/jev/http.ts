/**
 * POSTs to TypeSafe System One: one call with the retry policy the API
 * asks for, and a pool that sends many calls under one concurrency cap
 * and one rate-limit breaker.
 *
 * Guarantees: nothing here throws for a failed call. A call resolves the
 * answers map and both usage counts, or a fallback string the caller
 * reports as-is. 429 and 529 retry with exponential backoff; every other
 * failure ends the call at once. A failed call never stops the pool; only
 * an exhausted 429/529, the caller's signal, or the caller's own stop does.
 *
 * Intentionally not in the Log: the requests, the responses, and the
 * retries. The scorer turns answers into decisions; the compiler logs
 * those and the usage on the manifest.
 */

import { finiteOr, isRecord } from "@sage/core";
import { type Sleeper, sleep } from "@sage/provider/contract";
import { startDeadline } from "../deadline.js";

/** Retries after a 429 or 529 before the call falls back. */
const MAX_RETRIES = 3;
/**
 * First backoff in milliseconds when the response has no usable
 * `retry-after`; retry n waits `RETRY_BASE_MS × 2^n`.
 */
const RETRY_BASE_MS = 500;
/**
 * Longest `retry-after` honoured, in milliseconds. A longer server hint is
 * cut to this so one scorer call cannot stall a turn indefinitely; the
 * caller's signal still ends the wait earlier.
 */
const MAX_RETRY_AFTER_MS = 30_000;

/** System One origin used when no base URL is given. */
const DEFAULT_BASE_URL = "https://api.typesafe.ai";
/**
 * Requests in flight at once. The API limits each key to 100K tokens and
 * 80 requests per second; four requests of at most ~20KB stay well under
 * both, and a 429 still backs off per `retry-after`.
 */
const MAX_PARALLEL_REQUESTS = 4;

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

/**
 * A finished call: the answers map keyed by question id, and the
 * response's `usage.input_tokens` and `usage.output_tokens` (0 when
 * absent). A failure has `rateLimited: true` only when a 429 or 529
 * outlasted every retry; the scorer reads that flag, never the
 * `fallback` text.
 */
export type Posted =
  | {
      ok: true;
      answers: Record<string, unknown>;
      inputTokens: number;
      outputTokens: number;
    }
  | { ok: false; fallback: string; rateLimited?: true };

/** Where and how `postMany` sends. */
export type JevEndpoint = {
  apiKey: string;
  /** Origin of `/v1/systemone`; default `https://api.typesafe.ai`. */
  baseUrl?: string;
  /** Per-attempt timeout in milliseconds. */
  timeoutMs: number;
  /** Aborts the calls in flight and keeps new ones from starting. */
  signal?: AbortSignal;
  /** Test seam. Production uses the global `fetch`. */
  fetchImpl?: JevFetch;
  /** Test seam. Production uses `sleep` from the provider contract. */
  sleepImpl?: Sleeper;
};

/** Caller hooks into a `postMany` pass. */
export type PostHooks = {
  /** Checked before each call starts; true leaves the rest unsent. */
  shouldStop?: () => boolean;
  /** Runs once per call actually sent, in completion order. */
  onPosted?: (index: number, posted: Posted) => void;
};

/**
 * Sends every body, `MAX_PARALLEL_REQUESTS` at a time, and resolves one
 * slot per body in input order. A failed call costs only its own slot:
 * the others still go out. A body never sent is `undefined` when
 * `shouldStop` held it back, else a failure without a call: `aborted`
 * once `endpoint.signal` fired, or the fallback of the call that opened
 * the breaker. The breaker opens once any call has exhausted its 429/529
 * retries (`rateLimited`), which bounds the wall clock a rate-limited
 * pass can spend; a first 429 does not open it. Calls in flight finish
 * either way.
 */
export async function postMany(
  bodies: readonly string[],
  endpoint: JevEndpoint,
  hooks: PostHooks = {},
): Promise<(Posted | undefined)[]> {
  const target: PostTarget = {
    fetchImpl: endpoint.fetchImpl ?? defaultFetch,
    sleepImpl: endpoint.sleepImpl ?? sleep,
    url: `${(endpoint.baseUrl ?? DEFAULT_BASE_URL).replace(/\/+$/, "")}/v1/systemone`,
    apiKey: endpoint.apiKey,
    timeoutMs: endpoint.timeoutMs,
    signal: endpoint.signal,
  };
  const results: (Posted | undefined)[] = bodies.map(() => undefined);
  let rateLimit: string | undefined;
  let next = 0;
  const worker = async () => {
    while (next < bodies.length) {
      const index = next;
      next += 1;
      const body = bodies[index];
      if (body === undefined || hooks.shouldStop?.()) continue;
      const held = target.signal?.aborted ? "aborted" : rateLimit;
      if (held !== undefined) {
        results[index] = { ok: false, fallback: held };
        continue;
      }
      const posted = await postSystemOne(target, body);
      if (!posted.ok && posted.rateLimited) rateLimit ??= posted.fallback;
      results[index] = posted;
      hooks.onPosted?.(index, posted);
    }
  };
  const workers = Math.min(MAX_PARALLEL_REQUESTS, bodies.length);
  await Promise.all(Array.from({ length: workers }, worker));
  return results;
}

/** What one call needs besides the body. */
type PostTarget = {
  fetchImpl: JevFetch;
  sleepImpl: Sleeper;
  url: string;
  apiKey: string;
  timeoutMs: number;
  signal: AbortSignal | undefined;
};

/**
 * POSTs one body and resolves the answers or a fallback. Fallbacks:
 * `http:<status>` for a non-2xx status (429 and 529 only after
 * `MAX_RETRIES` retries; each wait is the response's `retry-after`, in
 * seconds or as an HTTP date and capped at `MAX_RETRY_AFTER_MS`, else
 * exponential), `timeout` when one attempt outlives `timeoutMs`,
 * `aborted` when the caller's signal fires (including during a backoff
 * wait), `invalid-json` for a body without an `answers` object, and
 * `error` when fetch rejects for any other reason.
 */
async function postSystemOne(
  target: PostTarget,
  body: string,
): Promise<Posted> {
  for (let attempt = 0; attempt <= MAX_RETRIES; attempt += 1) {
    const result = await attemptOnce(target, body);
    if (result.kind === "fallback") {
      return { ok: false, fallback: result.fallback };
    }
    if (result.status === 429 || result.status === 529) {
      if (attempt === MAX_RETRIES) {
        return {
          ok: false,
          fallback: `http:${result.status}`,
          rateLimited: true,
        };
      }
      const wait = result.retryAfterMs ?? RETRY_BASE_MS * 2 ** attempt;
      await target.sleepImpl(wait, target.signal);
      if (target.signal?.aborted) return { ok: false, fallback: "aborted" };
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

async function attemptOnce(
  target: PostTarget,
  body: string,
): Promise<
  | {
      kind: "http";
      status: number;
      text: string;
      retryAfterMs: number | undefined;
    }
  | { kind: "fallback"; fallback: string }
> {
  const deadline = startDeadline(target.timeoutMs, target.signal);
  try {
    const response = await target.fetchImpl(target.url, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${target.apiKey}`,
        "Content-Type": "application/json",
      },
      body,
      signal: deadline.signal,
    });
    const cause = deadline.cause();
    if (cause) return { kind: "fallback", fallback: cause };
    return {
      kind: "http",
      status: response.status,
      text: await response.text(),
      retryAfterMs: retryAfterMs(response.headers.get("retry-after")),
    };
  } catch {
    // fetch and body reads reject on abort and on network faults alike;
    // the deadline tells the two apart.
    return { kind: "fallback", fallback: deadline.cause() ?? "error" };
  }
}

/**
 * The `retry-after` header as a wait in milliseconds: delay-seconds or an
 * HTTP date, clamped to [0, `MAX_RETRY_AFTER_MS`]; undefined when absent
 * or unparseable.
 */
function retryAfterMs(header: string | null): number | undefined {
  if (header === null || header.trim() === "") return undefined;
  const text = header.trim();
  const ms = /^\d+(?:\.\d+)?$/.test(text)
    ? Number(text) * 1000
    : Date.parse(text) - Date.now();
  if (!Number.isFinite(ms)) return undefined;
  return Math.min(Math.max(ms, 0), MAX_RETRY_AFTER_MS);
}

function parseBody(text: string):
  | {
      answers: Record<string, unknown>;
      inputTokens: number;
      outputTokens: number;
    }
  | undefined {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    // Not JSON: the caller reports `invalid-json`.
    return undefined;
  }
  if (!isRecord(parsed) || !isRecord(parsed.answers)) return undefined;
  const usage = isRecord(parsed.usage) ? parsed.usage : {};
  return {
    answers: parsed.answers,
    inputTokens: finiteOr(usage.input_tokens, 0),
    outputTokens: finiteOr(usage.output_tokens, 0),
  };
}

function defaultFetch(
  url: string,
  init?: Parameters<JevFetch>[1],
): Promise<Response> {
  return globalThis.fetch(url, init);
}
