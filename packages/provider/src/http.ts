/**
 * POST JSON and classify the response. Non-2xx and transport failures stay
 * data; callers turn them into finish chunks.
 *
 * Deliberately not a Log entry: the API key, the abort signal, and the
 * response bytes never leave this module as a fact to append.
 */

import { asRecord } from "./record.js";

export type ClientOptions = {
  apiKey: string;
  baseUrl?: string;
  fetchImpl?: typeof fetch;
};

export function joinUrl(base: string, path: string): string {
  const root = base.endsWith("/") ? base.slice(0, -1) : base;
  return `${root}${path}`;
}

export type PostOk = { ok: true; body: ReadableStream<Uint8Array> };

export type PostFail = {
  ok: false;
  aborted: boolean;
  message: string;
  status?: number;
  raw?: unknown;
};

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
    if (args.signal?.aborted || isAbortError(error)) {
      return { ok: false, aborted: true, message: "aborted" };
    }
    return { ok: false, aborted: false, message: messageOf(error) };
  }
  if (!response.ok) {
    return readFailure(response);
  }
  if (!response.body) {
    return {
      ok: false,
      aborted: false,
      message: "stream ended before completion",
    };
  }
  return { ok: true, body: response.body };
}

async function readFailure(response: Response): Promise<PostFail> {
  const text = await response.text();
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

export function isAbortError(error: unknown): boolean {
  return error instanceof Error && error.name === "AbortError";
}

export function messageOf(error: unknown): string {
  if (error instanceof Error && error.message.length > 0) return error.message;
  return "stream ended before completion";
}
