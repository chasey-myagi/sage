/**
 * One POST to TypeSafe System One, with the retry policy the API asks for.
 *
 * Guarantees: `postSystemOne` never throws. It resolves the answers map and
 * both usage counts, or a fallback string the scorer reports as-is. 429
 * and 529 retry with exponential backoff; every other failure ends the
 * call at once.
 *
 * Intentionally not in the Log: the request, the response, and the
 * retries. The scorer turns answers into decisions; the compiler logs
 * those and the usage on the manifest.
 */

import { finiteOr, isRecord } from "@sage/core";
import type { Sleeper } from "@sage/provider/contract";
import { startDeadline } from "../deadline.js";

/** Retries after a 429 or 529 before the call falls back. */
const MAX_RETRIES = 3;
/** First backoff in milliseconds; retry n waits `RETRY_BASE_MS × 2^n`. */
const RETRY_BASE_MS = 500;

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
 * absent).
 */
export type Posted =
  | {
      ok: true;
      answers: Record<string, unknown>;
      inputTokens: number;
      outputTokens: number;
    }
  | { ok: false; fallback: string };

/** What one call needs besides the body. */
export type PostTarget = {
  fetchImpl: JevFetch;
  sleepImpl: Sleeper;
  url: string;
  apiKey: string;
  timeoutMs: number;
  signal: AbortSignal | undefined;
};

/**
 * POSTs `body` and resolves the answers or a fallback. Fallbacks:
 * `http:<status>` for a non-2xx status (429 and 529 only after
 * `MAX_RETRIES` retries), `timeout` when one attempt outlives `timeoutMs`,
 * `aborted` when the caller's signal fires (including during a backoff
 * wait), `invalid-json` for a body without an `answers` object, and
 * `error` when fetch rejects for any other reason.
 */
export async function postSystemOne(
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
        return { ok: false, fallback: `http:${result.status}` };
      }
      await target.sleepImpl(RETRY_BASE_MS * 2 ** attempt, target.signal);
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
  | { kind: "http"; status: number; text: string }
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
    };
  } catch {
    // fetch and body reads reject on abort and on network faults alike;
    // the deadline tells the two apart.
    return { kind: "fallback", fallback: deadline.cause() ?? "error" };
  }
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
