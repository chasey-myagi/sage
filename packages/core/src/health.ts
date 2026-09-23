/**
 * Run health: counts and token sums read back from one Log path.
 *
 * Guarantees: `runHealth` is a pure function of the path. It reads only
 * assistant `details.usage`, manifest `budget`, `scorer`, `shadow`, and
 * `entryScored`, `view_epoch`, `model_change`, `turn_retry`, tool_result
 * error flags, and `details.toolResults`; anything malformed counts as 0.
 * The same object is stored on the terminal entry.
 *
 * Intentionally not in the Log: this function. It does not copy claim
 * text, chunk text, reasons, paths, or child session ids.
 */

import {
  addUsage,
  emptyUsage,
  inputTokens,
  isRecord,
  type TokenCounts,
  totalTokens,
} from "@sage/provider/contract";
import { viewEpochs } from "./log/path.js";
import type { StoredEntry } from "./log/types.js";

/**
 * Token sums in run-health naming. `input` is uncached input only
 * (`inputOther`); `total` adds cache read, cache creation, and output.
 * Persisted inside `terminal.health`, so the field names are fixed.
 */
export type HealthTokens = {
  input: number;
  cacheRead: number;
  cacheCreation: number;
  output: number;
  total: number;
};

/** Health of one run. Every field is derived from the Log path alone. */
export type RunHealth = {
  /** Assistant messages on the path. */
  turns: number;
  /** Sum of assistant usage. */
  tokens: HealthTokens;
  /**
   * Σ cacheRead / Σ (cacheRead + input + cacheCreation) over assistant
   * usage; null when no assistant usage exists or the denominator is 0.
   */
  cacheHitRatio: number | null;
  /** `expands / omitted`; null when `omitted` is 0. */
  expandRate: number | null;
  /** The last manifest's `budget.omitted`. */
  omitted: number;
  /** The last manifest's `budget.expands` (already cumulative). */
  expands: number;
  /** One more than the highest `view_epoch.epoch`, or 1 when none exist. */
  epochs: number;
  /** model_change entries on the path. */
  modelChanges: number;
  /** Sum of manifest `scorer.calls`: scorer requests actually sent. */
  scorerCalls: number;
  /** Sum of numeric manifest `scorer.fallback`; `true` counts as 1. */
  scorerFallbacks: number;
  /** Manifests carrying a `shadow` object, and the sum of their usage. */
  shadow: {
    calls: number;
    usage: HealthTokens;
  };
  /**
   * Sums of each manifest's `entryScored` counts and usage. A manifest
   * without the object, or a missing `summaryRejected`, adds nothing.
   */
  entryScored: {
    calls: number;
    none: number;
    summary: number;
    full: number;
    summaryRejected: number;
    usage: HealthTokens;
  };
  /**
   * Child sessions spawned by `spawn_subtask` (one each) and
   * `spawn_subtasks` (one per item in `details.results`), and their summed
   * usage, read from `details.toolResults` on user messages. Not part of
   * `cacheHitRatio` or `expandRate`.
   */
  subtasks: {
    count: number;
    tokens: HealthTokens;
  };
  /** tool_result blocks with `isError: true`. Errors do not end the run. */
  toolErrors: number;
  /** turn_retry entries on the path. */
  turnRetries: number;
};

/** Computes `RunHealth` for `path`. Never throws on malformed entries. */
export function runHealth(path: readonly StoredEntry[]): RunHealth {
  let tokens: TokenCounts = emptyUsage();
  let turns = 0;
  let sawUsage = false;
  let omitted = 0;
  let expands = 0;
  let modelChanges = 0;
  let scorerCalls = 0;
  let scorerFallbacks = 0;

  for (const entry of path) {
    if (entry.kind === "message" && isAssistant(entry)) {
      turns += 1;
      const usage = readTokenCounts(entry.details?.usage);
      if (!usage) continue;
      sawUsage = true;
      tokens = addUsage(tokens, usage);
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
    if (entry.kind === "model_change") modelChanges += 1;
  }

  const inputMass = inputTokens(tokens);
  const epochs = viewEpochs(path);
  return {
    turns,
    tokens: toHealthTokens(tokens),
    cacheHitRatio:
      !sawUsage || inputMass === 0 ? null : tokens.inputCacheRead / inputMass,
    expandRate: omitted === 0 ? null : expands / omitted,
    omitted,
    expands,
    epochs: epochs.length > 0 ? Math.max(...epochs) + 1 : 1,
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
  let tokens: TokenCounts = emptyUsage();
  let calls = 0;
  for (const entry of path) {
    if (entry.kind !== "manifest" || !isRecord(entry.payload)) continue;
    if (!isRecord(entry.payload.shadow)) continue;
    calls += 1;
    tokens = addLogged(tokens, entry.payload.shadow.usage);
  }
  return { calls, usage: toHealthTokens(tokens) };
}

function entryHealth(path: readonly StoredEntry[]): RunHealth["entryScored"] {
  let tokens: TokenCounts = emptyUsage();
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
    tokens = addLogged(tokens, body.usage);
  }
  return {
    calls,
    none,
    summary,
    full,
    summaryRejected,
    usage: toHealthTokens(tokens),
  };
}

function subtaskHealth(path: readonly StoredEntry[]): RunHealth["subtasks"] {
  let tokens: TokenCounts = emptyUsage();
  let count = 0;
  const names = new Map<string, string>();
  for (const entry of path) {
    if (entry.kind !== "message" || !isRecord(entry.payload)) continue;
    recordToolNames(entry.payload.content, names);
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
      const spawned = absorbSpawn(named, details);
      count += spawned.count;
      tokens = addUsage(tokens, spawned.tokens);
    }
  }
  return { count, tokens: toHealthTokens(tokens) };
}

/** Adds each well-formed tool_use block's id → name to `names`. */
function recordToolNames(content: unknown, names: Map<string, string>): void {
  if (!Array.isArray(content)) return;
  for (const block of content) {
    if (!isRecord(block) || block.type !== "tool_use") continue;
    if (typeof block.id !== "string" || typeof block.name !== "string") {
      continue;
    }
    names.set(block.id, block.name);
  }
}

function absorbSpawn(
  name: string,
  details: Record<string, unknown>,
): { count: number; tokens: TokenCounts } {
  let tokens: TokenCounts = emptyUsage();
  if (name === "spawn_subtasks" && Array.isArray(details.results)) {
    let count = 0;
    for (const result of details.results) {
      if (!isRecord(result)) continue;
      tokens = addLogged(tokens, result.usage);
      count += 1;
    }
    return { count, tokens };
  }
  if (!isRecord(details.usage)) return { count: 0, tokens };
  return { count: 1, tokens: addLogged(tokens, details.usage) };
}

/**
 * Reads a logged usage object. Returns undefined for a non-object; a missing
 * or non-finite count reads as 0.
 */
function readTokenCounts(value: unknown): TokenCounts | undefined {
  if (!isRecord(value)) return undefined;
  return {
    inputOther: finite(value.inputOther) ?? 0,
    inputCacheRead: finite(value.inputCacheRead) ?? 0,
    inputCacheCreation: finite(value.inputCacheCreation) ?? 0,
    output: finite(value.output) ?? 0,
  };
}

/** Adds a logged usage object to `tokens`; a non-object adds nothing. */
function addLogged(tokens: TokenCounts, logged: unknown): TokenCounts {
  const usage = readTokenCounts(logged);
  return usage ? addUsage(tokens, usage) : tokens;
}

function toHealthTokens(tokens: TokenCounts): HealthTokens {
  return {
    input: tokens.inputOther,
    cacheRead: tokens.inputCacheRead,
    cacheCreation: tokens.inputCacheCreation,
    output: tokens.output,
    total: totalTokens(tokens),
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
