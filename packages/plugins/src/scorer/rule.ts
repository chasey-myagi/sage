/**
 * Deterministic scorer. Every candidate stays full.
 *
 * Contract: `name` is `rule` and `isNoop` is true. `score` returns one full decision per
 * candidate and omits both `usage` and `fallback`. It does not read
 * `prevRequest` or `query`.
 *
 * Intentionally not in the Log: this function. The compiler leaves
 * eliminate reasons unchanged when this scorer is installed.
 */

import type { Scorer } from "./types.js";

export const ruleScorer: Scorer = {
  name: "rule",
  isNoop: true,
  score(input) {
    return Promise.resolve({
      decisions: input.candidates.map((chunk) => ({
        id: chunk.id,
        tier: "full" as const,
      })),
    });
  },
};
