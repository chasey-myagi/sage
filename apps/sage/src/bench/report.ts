/**
 * `sage bench report <runA> <runB>`: compares two bench result directories.
 *
 * Contract: prints one JSON object `{ a, b, delta }`. `a` and `b` are
 * {@link BenchMetrics} plus that run's `budget`; `delta` is B minus A, null
 * where either side is null. Rates are counts over the task list.
 * `tokensPerResolved` is the sum of `health.tokens.total` over the resolved
 * count; `cacheHitRatio` and `expandRate` are means of the non-null
 * per-task values. Results files are read leniently: unknown fields fall
 * back to defaults and a malformed task row counts as an error.
 *
 * Intentionally not in the Log: the report. It reads health back off
 * `results.json`, which was computed from the copied sessions.
 */

import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import {
  DEFAULT_MAX_TURNS,
  errorText,
  isRecord,
  type RunHealth,
} from "@sage/core";
import { isFiniteNumber } from "../guards.js";
import { benchUsage } from "./args.js";
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
  const [left, right] = argv;
  if (
    argv.length !== 2 ||
    argv.some((arg) => arg.startsWith("-")) ||
    !left ||
    !right
  ) {
    io.stderr.write(`${benchUsage()}\n`);
    return 1;
  }
  try {
    const a = loadResults(path.resolve(io.cwd, left));
    const b = loadResults(path.resolve(io.cwd, right));
    const metricsA = benchMetrics(a);
    const metricsB = benchMetrics(b);
    const report = {
      a: { ...metricsA, budget: a.budget },
      b: { ...metricsB, budget: b.budget },
      delta: deltaOf(metricsA, metricsB),
    };
    io.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
    return 0;
  } catch (error) {
    io.stderr.write(`${errorText(error)}\n`);
    return 1;
  }
}

/** Aggregate rates for one run. */
export function benchMetrics(results: BenchResults): BenchMetrics {
  const tasks = results.tasks;
  const total = tasks.length;
  const resolved = tasks.filter((task) => task.status === "resolved").length;
  const empty = tasks.filter((task) => task.status === "empty-patch").length;
  const error = tasks.filter((task) => task.status === "error").length;
  const tokens = tasks.reduce((sum, task) => sum + tokenTotal(task), 0);
  return {
    resolvedRate: total === 0 ? null : resolved / total,
    emptyPatchRate: total === 0 ? null : empty / total,
    errorRate: total === 0 ? null : error / total,
    tokensPerResolved: resolved === 0 ? null : tokens / resolved,
    cacheHitRatio: mean(tasks.map((task) => task.health?.cacheHitRatio)),
    expandRate: mean(tasks.map((task) => task.health?.expandRate)),
  };
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
    budget: isFiniteNumber(parsed.budget) ? parsed.budget : null,
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
    };
  }
  const reason = typeof value.reason === "string" ? value.reason : undefined;
  return {
    id: value.id,
    status: benchStatus(value.status),
    ...(reason ? { reason } : {}),
    // Only the numeric fields read below are used, each re-checked there.
    health: isRecord(value.health) ? (value.health as RunHealth) : null,
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
    tokensPerResolved: subtract(a.tokensPerResolved, b.tokensPerResolved),
    cacheHitRatio: subtract(a.cacheHitRatio, b.cacheHitRatio),
    expandRate: subtract(a.expandRate, b.expandRate),
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
