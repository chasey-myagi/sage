/**
 * Run health read back from one log path.
 *
 * Contract: every number comes from assistant `details.usage`, the last
 * manifest `budget`, every manifest `scorer`, `view_epoch`, and
 * `model_change`. `tokens.input` is the sum of `inputOther`. `tokens.total`
 * adds cache read, cache creation, and output. `cacheHitRatio` is
 * Σ cacheRead / Σ (cacheRead + inputOther + cacheCreation), or null when
 * no assistant usage exists or that denominator is 0. `omitted` and
 * `expands` are the last manifest's budget fields (already cumulative).
 * `expandRate` is expands/omitted, or null when omitted is 0. `epochs` is
 * one more than the highest `view_epoch.epoch`, or 1 when none exist.
 * `scorerCalls` sums `scorer.calls`. `scorerFallbacks` sums numeric
 * `scorer.fallback`, and counts `true` as 1.
 *
 * Intentionally not in the Log: this function. It does not copy claim
 * text, chunk text, reasons, or paths.
 */

import { isRecord, type StoredEntry } from "./log/types.js";

export type RunHealth = {
  turns: number;
  tokens: {
    input: number;
    cacheRead: number;
    cacheCreation: number;
    output: number;
    total: number;
  };
  cacheHitRatio: number | null;
  expandRate: number | null;
  omitted: number;
  expands: number;
  epochs: number;
  modelChanges: number;
  scorerCalls: number;
  scorerFallbacks: number;
};

export function runHealth(path: readonly StoredEntry[]): RunHealth {
  const tokens = {
    input: 0,
    cacheRead: 0,
    cacheCreation: 0,
    output: 0,
  };
  let turns = 0;
  let sawUsage = false;
  let omitted = 0;
  let expands = 0;
  let highestEpoch = 0;
  let sawEpoch = false;
  let modelChanges = 0;
  let scorerCalls = 0;
  let scorerFallbacks = 0;

  for (const entry of path) {
    if (entry.kind === "message" && isAssistant(entry)) {
      turns += 1;
      const usage = entry.details?.usage;
      if (!isRecord(usage)) continue;
      sawUsage = true;
      tokens.input += finite(usage.inputOther) ?? 0;
      tokens.cacheRead += finite(usage.inputCacheRead) ?? 0;
      tokens.cacheCreation += finite(usage.inputCacheCreation) ?? 0;
      tokens.output += finite(usage.output) ?? 0;
      continue;
    }
    if (entry.kind === "manifest") {
      const budget = budgetOf(entry);
      omitted = budget?.omitted ?? 0;
      expands = budget?.expands ?? 0;
      const scorer = scorerOf(entry);
      scorerCalls += scorer.calls;
      scorerFallbacks += scorer.fallbacks;
      continue;
    }
    if (entry.kind === "view_epoch" && isRecord(entry.payload)) {
      const epoch = finite(entry.payload.epoch);
      if (epoch === undefined) continue;
      if (!sawEpoch || epoch > highestEpoch) highestEpoch = epoch;
      sawEpoch = true;
      continue;
    }
    if (entry.kind === "model_change") modelChanges += 1;
  }

  const inputMass = tokens.cacheRead + tokens.input + tokens.cacheCreation;
  const total =
    tokens.input + tokens.cacheRead + tokens.cacheCreation + tokens.output;
  return {
    turns,
    tokens: { ...tokens, total },
    cacheHitRatio:
      !sawUsage || inputMass === 0 ? null : tokens.cacheRead / inputMass,
    expandRate: omitted === 0 ? null : expands / omitted,
    omitted,
    expands,
    epochs: sawEpoch ? highestEpoch + 1 : 1,
    modelChanges,
    scorerCalls,
    scorerFallbacks,
  };
}

function isAssistant(entry: StoredEntry): boolean {
  return isRecord(entry.payload) && entry.payload.role === "assistant";
}

function budgetOf(
  entry: StoredEntry,
): { omitted: number; expands: number } | undefined {
  if (!isRecord(entry.payload)) return undefined;
  const budget = entry.payload.budget;
  if (!isRecord(budget)) return undefined;
  return {
    omitted: finite(budget.omitted) ?? 0,
    expands: finite(budget.expands) ?? 0,
  };
}

function scorerOf(entry: StoredEntry): { calls: number; fallbacks: number } {
  if (!isRecord(entry.payload) || !isRecord(entry.payload.scorer)) {
    return { calls: 0, fallbacks: 0 };
  }
  const scorer = entry.payload.scorer;
  return {
    calls: finite(scorer.calls) ?? 0,
    fallbacks: fallbackCount(scorer.fallback),
  };
}

function fallbackCount(value: unknown): number {
  if (value === true) return 1;
  const numeric = finite(value);
  return numeric ?? 0;
}

function finite(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}
