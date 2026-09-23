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
 * `scorer.fallback`, and counts `true` as 1. `shadow.calls` counts
 * manifests that carry a `shadow` object. `shadow.usage` sums that
 * object's `usage` quadruple (`inputOther`, `inputCacheRead`,
 * `inputCacheCreation`, `output`). `entryScored` sums each manifest's
 * `entryScored` object: `calls`, `none`, `summary`, `full`,
 * `summaryRejected`, and the same usage quadruple. A missing
 * `summaryRejected` adds nothing. A manifest without the object adds
 * nothing. `subtasks` sums usage from
 * `spawn_subtask` and `spawn_subtasks` tool results stored on the user
 * message as `details.toolResults`. `spawn_subtasks` counts each item in
 * `details.results`. `cacheHitRatio` and `expandRate` ignore that usage.
 * `toolErrors` counts `tool_result` blocks whose `isError` is true. The
 * count does not end the run. `turnRetries` counts `turn_retry` entries.
 *
 * Intentionally not in the Log: this function. It does not copy claim
 * text, chunk text, reasons, paths, or child session ids.
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
  shadow: {
    calls: number;
    usage: {
      input: number;
      cacheRead: number;
      cacheCreation: number;
      output: number;
      total: number;
    };
  };
  entryScored: {
    calls: number;
    none: number;
    summary: number;
    full: number;
    summaryRejected: number;
    usage: {
      input: number;
      cacheRead: number;
      cacheCreation: number;
      output: number;
      total: number;
    };
  };
  subtasks: {
    count: number;
    tokens: {
      input: number;
      cacheRead: number;
      cacheCreation: number;
      output: number;
      total: number;
    };
  };
  toolErrors: number;
  turnRetries: number;
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
    shadow: shadowHealth(path),
    entryScored: entryHealth(path),
    subtasks: subtaskHealth(path),
    toolErrors: toolErrorCount(path),
    turnRetries: turnRetryCount(path),
  };
}

function turnRetryCount(path: readonly StoredEntry[]): number {
  let count = 0;
  for (const entry of path) {
    if (entry.kind === "turn_retry") count += 1;
  }
  return count;
}

function toolErrorCount(path: readonly StoredEntry[]): number {
  let count = 0;
  for (const entry of path) {
    if (entry.kind !== "message" || !isRecord(entry.payload)) continue;
    const content = entry.payload.content;
    if (!Array.isArray(content)) continue;
    for (const block of content) {
      if (!isRecord(block) || block.type !== "tool_result") continue;
      if (block.isError === true) count += 1;
    }
  }
  return count;
}

function shadowHealth(path: readonly StoredEntry[]): RunHealth["shadow"] {
  const tokens = {
    input: 0,
    cacheRead: 0,
    cacheCreation: 0,
    output: 0,
  };
  let calls = 0;
  for (const entry of path) {
    if (entry.kind !== "manifest" || !isRecord(entry.payload)) continue;
    if (!isRecord(entry.payload.shadow)) continue;
    calls += 1;
    addUsage(tokens, entry.payload.shadow.usage);
  }
  const total =
    tokens.input + tokens.cacheRead + tokens.cacheCreation + tokens.output;
  return { calls, usage: { ...tokens, total } };
}

function entryHealth(path: readonly StoredEntry[]): RunHealth["entryScored"] {
  const tokens = {
    input: 0,
    cacheRead: 0,
    cacheCreation: 0,
    output: 0,
  };
  let calls = 0;
  let none = 0;
  let summary = 0;
  let full = 0;
  let summaryRejected = 0;
  for (const entry of path) {
    if (entry.kind !== "manifest" || !isRecord(entry.payload)) continue;
    const body = entry.payload.entryScored;
    if (!isRecord(body)) continue;
    calls += finite(body.calls) ?? 0;
    none += finite(body.none) ?? 0;
    summary += finite(body.summary) ?? 0;
    full += finite(body.full) ?? 0;
    summaryRejected += finite(body.summaryRejected) ?? 0;
    addUsage(tokens, body.usage);
  }
  const total =
    tokens.input + tokens.cacheRead + tokens.cacheCreation + tokens.output;
  return {
    calls,
    none,
    summary,
    full,
    summaryRejected,
    usage: { ...tokens, total },
  };
}

function subtaskHealth(path: readonly StoredEntry[]): RunHealth["subtasks"] {
  const tokens = {
    input: 0,
    cacheRead: 0,
    cacheCreation: 0,
    output: 0,
  };
  let count = 0;
  const names = new Map<string, string>();
  for (const entry of path) {
    if (entry.kind !== "message" || !isRecord(entry.payload)) continue;
    const content = entry.payload.content;
    if (Array.isArray(content)) {
      for (const block of content) {
        if (!isRecord(block) || block.type !== "tool_use") continue;
        if (typeof block.id !== "string" || typeof block.name !== "string") {
          continue;
        }
        names.set(block.id, block.name);
      }
    }
    const listed = entry.details?.toolResults;
    if (!Array.isArray(listed)) continue;
    for (const item of listed) {
      if (!isRecord(item)) continue;
      const named =
        typeof item.name === "string"
          ? item.name
          : typeof item.toolUseId === "string"
            ? names.get(item.toolUseId)
            : undefined;
      if (named !== "spawn_subtask" && named !== "spawn_subtasks") continue;
      const details = isRecord(item.details) ? item.details : undefined;
      if (!details) continue;
      count += absorbSpawn(named, details, tokens);
    }
  }
  const total =
    tokens.input + tokens.cacheRead + tokens.cacheCreation + tokens.output;
  return { count, tokens: { ...tokens, total } };
}

function absorbSpawn(
  name: string,
  details: Record<string, unknown>,
  tokens: {
    input: number;
    cacheRead: number;
    cacheCreation: number;
    output: number;
  },
): number {
  if (name === "spawn_subtasks" && Array.isArray(details.results)) {
    let count = 0;
    for (const result of details.results) {
      if (!isRecord(result)) continue;
      addUsage(tokens, result.usage);
      count += 1;
    }
    return count;
  }
  if (!isRecord(details.usage)) return 0;
  addUsage(tokens, details.usage);
  return 1;
}

function addUsage(
  tokens: {
    input: number;
    cacheRead: number;
    cacheCreation: number;
    output: number;
  },
  usage: unknown,
): void {
  if (!isRecord(usage)) return;
  tokens.input += finite(usage.inputOther) ?? 0;
  tokens.cacheRead += finite(usage.inputCacheRead) ?? 0;
  tokens.cacheCreation += finite(usage.inputCacheCreation) ?? 0;
  tokens.output += finite(usage.output) ?? 0;
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
