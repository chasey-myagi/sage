/**
 * Retries a POST until the response body starts or the attempts run out.
 *
 * Guarantees: fetch throws, HTTP 429, and HTTP 5xx wait and try again; any
 * other 4xx does not. This module never sees a body that has already
 * begun. The final failure keeps postJson's `transport` flag for the
 * finish chunk.
 *
 * Intentionally not in the Log: failed response bodies, the sleep, and the
 * API key. The retry count and the last failure reason ride on the usage
 * object the caller already logs. No new entry kind.
 */

import { backoffMs, type Sleeper, sleep } from "./contract/backoff.js";
import type { PostFail, PostOk } from "./http.js";
import { postJson } from "./http.js";

/** Retries after the first attempt, and the first delay in milliseconds. */
export type RetryPolicy = {
  attempts: number;
  baseMs: number;
};

/** Two retries, 1s then 4s. Core's whole-turn retry uses its own numbers. */
const DEFAULT_RETRY: RetryPolicy = { attempts: 2, baseMs: 1000 };

/** Result of `openWithRetry`: an open body, an abort, or the final failure. */
export type OpenedCall =
  | {
      ok: true;
      body: ReadableStream<Uint8Array>;
      retries: number;
      reason?: string;
    }
  | { ok: false; aborted: true; retries: number; reason?: string }
  | {
      ok: false;
      aborted: false;
      message: string;
      status?: number;
      raw?: unknown;
      transport?: boolean;
      retries: number;
      reason: string;
    };

function isRetryable(fail: PostFail): boolean {
  if (fail.aborted) return false;
  if (fail.transport) return true;
  if (fail.status === 429) return true;
  return fail.status !== undefined && fail.status >= 500;
}

/**
 * Opens the response body, retrying as above. Never throws. `retries`
 * counts waits taken; `reason` is the last retried failure. An abort, before
 * or between attempts, returns `aborted: true`.
 */
export async function openWithRetry(args: {
  url: string;
  headers: Record<string, string>;
  json: unknown;
  signal?: AbortSignal;
  fetchImpl?: typeof fetch;
  retry?: RetryPolicy;
  sleep?: Sleeper;
}): Promise<OpenedCall> {
  const attempts = args.retry?.attempts ?? DEFAULT_RETRY.attempts;
  const baseMs = args.retry?.baseMs ?? DEFAULT_RETRY.baseMs;
  const wait = args.sleep ?? sleep;
  let retries = 0;
  let reason: string | undefined;

  while (true) {
    if (args.signal?.aborted) {
      return { ok: false, aborted: true, retries, reason };
    }
    const opened = await postJson({
      url: args.url,
      headers: args.headers,
      json: args.json,
      signal: args.signal,
      fetchImpl: args.fetchImpl,
    });
    if (opened.ok) return succeed(opened, retries, reason);
    if (opened.aborted) return { ok: false, aborted: true, retries, reason };
    if (!isRetryable(opened) || retries >= attempts) {
      return fail(opened, retries);
    }
    retries += 1;
    reason = opened.message;
    await wait(backoffMs(retries, baseMs), args.signal);
  }
}

function succeed(
  opened: PostOk,
  retries: number,
  reason: string | undefined,
): OpenedCall {
  if (reason !== undefined) {
    return { ok: true, body: opened.body, retries, reason };
  }
  return { ok: true, body: opened.body, retries };
}

function fail(opened: PostFail, retries: number): OpenedCall {
  const result: Extract<OpenedCall, { ok: false; aborted: false }> = {
    ok: false,
    aborted: false,
    message: opened.message,
    retries,
    reason: opened.message,
  };
  if (opened.status !== undefined) result.status = opened.status;
  if (opened.raw !== undefined) result.raw = opened.raw;
  if (opened.transport) result.transport = true;
  return result;
}
