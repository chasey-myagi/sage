/**
 * Token usage every adapter reports, and the arithmetic on it.
 *
 * Guarantees: the four counts are the only source of token numbers; `raw`
 * is the provider payload kept for inspection, never a second source.
 *
 * Intentionally not in the Log by itself: the assistant entry copies a
 * `Usage` into its details. `details` on it is not model-visible and not a
 * separate entry kind.
 */

import { finiteOr, isRecord } from "./record.js";

/** The four token counts, without the provider payload or retry details. */
export type TokenCounts = {
  inputOther: number;
  inputCacheRead: number;
  inputCacheCreation: number;
  output: number;
};

/**
 * Usage of one call. `details` is the pre-stream retry count and the last
 * failure reason, omitted when the call was not retried.
 */
export type Usage = TokenCounts & {
  raw?: unknown;
  details?: {
    retries: number;
    reason: string;
  };
};

/** A fresh all-zero usage. Each call returns a new object the caller owns. */
export function emptyUsage(): Usage {
  return {
    inputOther: 0,
    inputCacheRead: 0,
    inputCacheCreation: 0,
    output: 0,
  };
}

/** Field-wise sum of the four counts. `raw` and `details` are dropped. */
export function addUsage(left: TokenCounts, right: TokenCounts): TokenCounts {
  return {
    inputOther: left.inputOther + right.inputOther,
    inputCacheRead: left.inputCacheRead + right.inputCacheRead,
    inputCacheCreation: left.inputCacheCreation + right.inputCacheCreation,
    output: left.output + right.output,
  };
}

/** Every input token: uncached, cache read, and cache creation. */
export function inputTokens(usage: TokenCounts): number {
  return usage.inputOther + usage.inputCacheRead + usage.inputCacheCreation;
}

/** Input plus output tokens. */
export function totalTokens(usage: TokenCounts): number {
  return inputTokens(usage) + usage.output;
}

/**
 * Reads the four counts of a usage object read back from the Log or another
 * untrusted source. Returns undefined for a non-object; a missing or
 * non-finite count reads as 0. `raw` and `details` are dropped.
 */
export function readTokenCounts(value: unknown): TokenCounts | undefined {
  if (!isRecord(value)) return undefined;
  return {
    inputOther: finiteOr(value.inputOther, 0),
    inputCacheRead: finiteOr(value.inputCacheRead, 0),
    inputCacheCreation: finiteOr(value.inputCacheCreation, 0),
    output: finiteOr(value.output, 0),
  };
}
