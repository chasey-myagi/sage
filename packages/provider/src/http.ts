/**
 * POSTs JSON and classifies the response.
 *
 * Guarantees: nothing here throws for a failed call. Non-2xx responses and
 * transport failures come back as data (`PostFail`). A fetch throw, or a
 * throw while reading a non-2xx body, is marked `transport: true`; callers
 * turn them into finish chunks.
 *
 * Intentionally not in the Log: the API key, the abort signal, and the
 * response bytes never leave this module as a fact to append.
 */

import type { Sleeper } from "./contract/backoff.js";
import { INCOMPLETE_STREAM } from "./contract/stream.js";
import { asRecord } from "./record.js";
import type { RetryPolicy } from "./retry.js";

/** Adapter constructor options. */
export type ClientOptions = {
  apiKey: string;
  baseUrl?: string;
  fetchImpl?: typeof fetch;
  /**
   * Retries before the response body starts. Omitted means 2 retries
   * and a 1000ms base (1s, then 4s). Not a Log entry.
   */
  retry?: RetryPolicy;
  /** Replaces the real timer. Tests pass this so they do not wait. */
  sleep?: Sleeper;
};

/** `ClientOptions` with the base URL resolved; what an adapter keeps. */
export type Client = Omit<ClientOptions, "baseUrl"> & { baseUrl: string };

/** Fills in `defaultBaseUrl` when `options.baseUrl` is absent. */
export function resolveClient(
  options: ClientOptions,
  defaultBaseUrl: string,
): Client {
  return { ...options, baseUrl: options.baseUrl ?? defaultBaseUrl };
}

/** `base` plus `path`, with at most one slash between them. */
export function joinUrl(base: string, path: string): string {
  const root = base.endsWith("/") ? base.slice(0, -1) : base;
  return `${root}${path}`;
}

/** The call reached a 2xx with a body; the caller now owns `body`. */
export type PostOk = { ok: true; body: ReadableStream<Uint8Array> };

/** A failed call. `raw` is the parsed error body (or its text), if any. */
export type PostFail = {
  ok: false;
  aborted: boolean;
  message: string;
  status?: number;
  raw?: unknown;
  /** True when fetch itself threw. HTTP responses leave this unset. */
  transport?: boolean;
};

/**
 * One POST. The error message comes from the body's `error.message` or
 * `message` when present, else the body text, else `HTTP <status>`.
 */
export async function postJson(args: {
  url: string;
  headers: Record<string, string>;
  json: unknown;
  signal?: AbortSignal;
  fetchImpl?: typeof fetch;
}): Promise<PostOk | PostFail> {
  if (args.signal?.aborted) {
    return { ok: false, aborted: true, message: "aborted" };
  }
  const fetchImpl = args.fetchImpl ?? fetch;
  let response: Response;
  try {
    response = await fetchImpl(args.url, {
      method: "POST",
      headers: args.headers,
      body: JSON.stringify(args.json),
      signal: args.signal,
    });
  } catch (error) {
    return thrownFailure(error, args.signal);
  }
  if (!response.ok) {
    return readFailure(response, args.signal);
  }
  if (!response.body) {
    return {
      ok: false,
      aborted: false,
      message: INCOMPLETE_STREAM,
    };
  }
  return { ok: true, body: response.body };
}

/**
 * Classifies an exception thrown by fetch or by a body read: an abort stays
 * an abort, anything else is a transport failure the caller may retry.
 */
function thrownFailure(error: unknown, signal?: AbortSignal): PostFail {
  if (signal?.aborted || isAbortError(error)) {
    return { ok: false, aborted: true, message: "aborted" };
  }
  return {
    ok: false,
    aborted: false,
    message: messageOf(error),
    transport: true,
  };
}

async function readFailure(
  response: Response,
  signal?: AbortSignal,
): Promise<PostFail> {
  let text: string;
  try {
    text = await response.text();
  } catch (error) {
    // The connection dropped while the error body streamed in. That is a
    // failure below HTTP, so it carries no status, the same as a fetch throw.
    return thrownFailure(error, signal);
  }
  let raw: unknown;
  let message = `HTTP ${response.status}`;
  if (text.length > 0) {
    try {
      raw = JSON.parse(text) as unknown;
      message = messageFromPayload(raw) ?? message;
    } catch {
      raw = text;
      message = text;
    }
  }
  return {
    ok: false,
    aborted: false,
    message,
    status: response.status,
    raw,
  };
}

function messageFromPayload(raw: unknown): string | undefined {
  const record = asRecord(raw);
  if (!record) return undefined;
  const nested = asRecord(record.error);
  if (nested && typeof nested.message === "string") return nested.message;
  if (typeof record.message === "string") return record.message;
  return undefined;
}

/** True for the `AbortError` fetch throws when its signal fires. */
export function isAbortError(error: unknown): boolean {
  return error instanceof Error && error.name === "AbortError";
}

/** The error's message, or `INCOMPLETE_STREAM` when it has none. */
export function messageOf(error: unknown): string {
  if (error instanceof Error && error.message.length > 0) return error.message;
  return INCOMPLETE_STREAM;
}
