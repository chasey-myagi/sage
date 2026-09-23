/**
 * Usage quadruple shared by every adapter.
 *
 * Deliberately not a Log entry by itself. The assistant entry that lands
 * in the Log copies this object (#4). `raw` is the provider payload escape
 * hatch and is not a second source of the four numbers. `details` is the
 * pre-stream retry count and the last failure reason. It is omitted when
 * the call was not retried. It is not model-visible and not a new entry
 * kind.
 */

/** The four token counts, without the provider payload or retry details. */
export type TokenCounts = {
  inputOther: number;
  inputCacheRead: number;
  inputCacheCreation: number;
  output: number;
};

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
