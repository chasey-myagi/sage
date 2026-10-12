/**
 * `sage bench report [--exclude-faults] <runA> <runB>`: compares two bench
 * result directories.
 *
 * Contract: prints one JSON object `{ a, b, delta, excluded }`. `a` and `b`
 * are {@link BenchMetrics} plus that run's `budget`, `baseUrl`, and
 * `provider`; `delta` is B minus A, null where either side is null. Rates
 * are counts over the compared tasks. With `--exclude-faults`, every task
 * either run faulted on (`isFaultRow`) is dropped from both sides before
 * any rate is computed and `excluded` lists those ids, each once, in A's
 * order then B's; without it `excluded` is empty and every task counts.
 * `tokensPerResolved` is the sum of `health.tokens.total` over the resolved
 * count; `cacheHitRatio` and `expandRate` are means of the non-null
 * per-task values; `scorerFailedRequests` is a sum over tasks with
 * health. Results files are read leniently: unknown fields fall back to
 * defaults and a malformed task row counts as an error.
 *
 * Intentionally not in the Log: the report. It reads health back off
 * `results.json`, which was computed from the copied sessions.
 */

import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import {
  DEFAULT_MAX_TURNS,
  errorText,
  finiteOr,
  isFiniteNumber,
  isRecord,
  type RunHealth,
} from "@sage/core";
import { benchUsage, parseReport } from "./args.js";
import { isFaultRow } from "./fault.js";
import type {
  BenchIo,
  BenchMetrics,
  BenchResults,
  BenchStatus,
  BenchTaskResult,
} from "./types.js";

/**
 * Runs `sage bench report <argv>`. Returns 0 after printing, 1 for a usage
 * error or a missing or unreadable `results.json`.
 */
export function runBenchReport(argv: string[], io: BenchIo): number {
  const parsed = parseReport(argv);
  if (!parsed) {
    io.stderr.write(`${benchUsage()}\n`);
    return 1;
  }
  try {
    const a = loadResults(path.resolve(io.cwd, parsed.left));
    const b = loadResults(path.resolve(io.cwd, parsed.right));
    const excluded = parsed.excludeFaults ? faultedIds(a.tasks, b.tasks) : [];
    const metricsA = benchMetrics(withoutIds(a.tasks, excluded));
    const metricsB = benchMetrics(withoutIds(b.tasks, excluded));
    const report = {
      a: { ...metricsA, ...runConfig(a) },
      b: { ...metricsB, ...runConfig(b) },
      delta: deltaOf(metricsA, metricsB),
      excluded,
    };
    io.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
    return 0;
  } catch (error) {
    io.stderr.write(`${errorText(error)}\n`);
    return 1;
  }
}

/** The run settings printed beside each side's metrics. */
function runConfig(
  results: BenchResults,
): Pick<BenchResults, "budget" | "baseUrl" | "provider"> {
  return {
    budget: results.budget,
    baseUrl: results.baseUrl,
    provider: results.provider,
  };
}

/**
 * Ids of the tasks either run faulted on, each once, in A's order then B's.
 * Dropping them from both sides keeps the comparison to the same tasks.
 */
function faultedIds(
  left: readonly BenchTaskResult[],
  right: readonly BenchTaskResult[],
): string[] {
  const ids = [...left, ...right].filter(isFaultRow).map((task) => task.id);
  return [...new Set(ids)];
}

function withoutIds(
  tasks: readonly BenchTaskResult[],
  ids: readonly string[],
): BenchTaskResult[] {
  const dropped = new Set(ids);
  return tasks.filter((task) => !dropped.has(task.id));
}

/** Aggregate rates over one run's compared tasks. */
function benchMetrics(tasks: readonly BenchTaskResult[]): BenchMetrics {
  const total = tasks.length;
  const resolved = tasks.filter((task) => task.status === "resolved").length;
  const empty = tasks.filter((task) => task.status === "empty-patch").length;
  const error = tasks.filter((task) => task.status === "error").length;
  const faults = tasks.filter(isFaultRow).length;
  const stopped = tasks.filter((task) => task.stoppedEarly === true).length;
  const tokens = tasks.reduce((sum, task) => sum + tokenTotal(task), 0);
  return {
    resolvedRate: total === 0 ? null : resolved / total,
    emptyPatchRate: total === 0 ? null : empty / total,
    errorRate: total === 0 ? null : error / total,
    faultRate: total === 0 ? null : faults / total,
    tokensPerResolved: resolved === 0 ? null : tokens / resolved,
    cacheHitRatio: mean(tasks.map((task) => task.health?.cacheHitRatio)),
    expandRate: mean(tasks.map((task) => task.health?.expandRate)),
    stoppedEarlyRate: total === 0 ? null : stopped / total,
    scorerFailedRequests: failedRequestTotal(tasks),
  };
}

/** Sum of each task's failed scorer requests; null when no task has health. */
function failedRequestTotal(tasks: readonly BenchTaskResult[]): number | null {
  let sum: number | null = null;
  for (const task of tasks) {
    const health = task.health;
    if (!health) continue;
    sum =
      (sum ?? 0) +
      finiteOr(health.scorerFailedRequests, 0) +
      finiteOr(health.entryScored?.failedRequests, 0) +
      finiteOr(health.shadow?.failedRequests, 0);
  }
  return sum;
}

function loadResults(dir: string): BenchResults {
  const file = path.join(dir, "results.json");
  if (!existsSync(file)) throw new Error(`Missing results: ${file}`);
  const parsed: unknown = JSON.parse(readFileSync(file, "utf8"));
  if (!isRecord(parsed) || !Array.isArray(parsed.tasks)) {
    throw new Error(`Invalid results: ${file}`);
  }
  const maxTurns = parsed.maxTurns;
  return {
    policy: parsed.policy === "passthrough" ? "passthrough" : "compiler",
    model: typeof parsed.model === "string" ? parsed.model : "",
    provider: typeof parsed.provider === "string" ? parsed.provider : null,
    budget: isFiniteNumber(parsed.budget) ? parsed.budget : null,
    baseUrl: typeof parsed.baseUrl === "string" ? parsed.baseUrl : null,
    apiKeyEnv: typeof parsed.apiKeyEnv === "string" ? parsed.apiKeyEnv : null,
    toolNetwork: parsed.toolNetwork === "full" ? "full" : "none",
    maxTurns:
      typeof maxTurns === "number" &&
      Number.isSafeInteger(maxTurns) &&
      maxTurns > 0
        ? maxTurns
        : DEFAULT_MAX_TURNS,
    scorerApply: parsed.scorerApply === "entry" ? "entry" : "rebuild",
    tasks: parsed.tasks.map((item) => readTaskResult(item)),
  };
}

function readTaskResult(value: unknown): BenchTaskResult {
  if (!isRecord(value) || typeof value.id !== "string") {
    return {
      id: "unknown",
      status: "error",
      reason: "invalid-result",
      health: null,
      stoppedEarly: null,
    };
  }
  const reason = typeof value.reason === "string" ? value.reason : undefined;
  return {
    id: value.id,
    status: benchStatus(value.status),
    ...(reason ? { reason } : {}),
    // Only the numeric fields read below are used, each re-checked there.
    health: isRecord(value.health) ? (value.health as RunHealth) : null,
    stoppedEarly:
      typeof value.stoppedEarly === "boolean" ? value.stoppedEarly : null,
  };
}

function benchStatus(value: unknown): BenchStatus {
  if (value === "resolved" || value === "empty-patch" || value === "error") {
    return value;
  }
  return "error";
}

function deltaOf(a: BenchMetrics, b: BenchMetrics): BenchMetrics {
  return {
    resolvedRate: subtract(a.resolvedRate, b.resolvedRate),
    emptyPatchRate: subtract(a.emptyPatchRate, b.emptyPatchRate),
    errorRate: subtract(a.errorRate, b.errorRate),
    faultRate: subtract(a.faultRate, b.faultRate),
    tokensPerResolved: subtract(a.tokensPerResolved, b.tokensPerResolved),
    cacheHitRatio: subtract(a.cacheHitRatio, b.cacheHitRatio),
    expandRate: subtract(a.expandRate, b.expandRate),
    stoppedEarlyRate: subtract(a.stoppedEarlyRate, b.stoppedEarlyRate),
    scorerFailedRequests: subtract(
      a.scorerFailedRequests,
      b.scorerFailedRequests,
    ),
  };
}

function subtract(left: number | null, right: number | null): number | null {
  if (left === null || right === null) return null;
  return right - left;
}

function tokenTotal(task: BenchTaskResult): number {
  const total = task.health?.tokens?.total;
  return isFiniteNumber(total) ? total : 0;
}

function mean(values: readonly (number | null | undefined)[]): number | null {
  const finite = values.filter(isFiniteNumber);
  if (finite.length === 0) return null;
  return finite.reduce((sum, value) => sum + value, 0) / finite.length;
}
