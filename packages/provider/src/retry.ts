/**
 * Retry a POST until the response body starts, or the budget runs out.
 * Fetch throws, HTTP 429, and HTTP 5xx wait and try again. Any other
 * 4xx does not. This module never sees a body that has already begun.
 *
 * Deliberately not a Log entry: failed response bodies, the sleep, and
 * the API key stay here. The retry count and the last failure reason
 * ride on the usage object the caller already logs. No new entry kind.
 */

import type { PostFail, PostOk } from "./http.js";
import { postJson } from "./http.js";

export type RetryPolicy = {
  attempts: number;
  baseMs: number;
};

export type Sleeper = (ms: number, signal?: AbortSignal) => Promise<void>;

const DEFAULT_RETRY: RetryPolicy = { attempts: 2, baseMs: 1000 };

type OpenedCall =
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
      retries: number;
      reason: string;
    };

function backoffMs(attempt: number, baseMs: number): number {
  return baseMs * 4 ** (attempt - 1);
}

function delay(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal?.aborted) {
      resolve();
      return;
    }
    let timer: ReturnType<typeof setTimeout> | undefined;
    const finish = (): void => {
      if (timer !== undefined) clearTimeout(timer);
      signal?.removeEventListener("abort", finish);
      resolve();
    };
    timer = setTimeout(finish, ms);
    signal?.addEventListener("abort", finish, { once: true });
  });
}

function isRetryable(fail: PostFail): boolean {
  if (fail.aborted) return false;
  if (fail.transport) return true;
  if (fail.status === 429) return true;
  return fail.status !== undefined && fail.status >= 500;
}

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
  const sleep = args.sleep ?? delay;
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
    await sleep(backoffMs(retries, baseMs), args.signal);
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
  return result;
}
