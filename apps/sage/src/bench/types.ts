/**
 * Types shared by the bench runner, its report, and its test seams.
 *
 * Contract: `BenchResults` is the shape of `<out>/results.json`. It holds
 * ids, statuses, reasons, and health numbers, never problem text, test
 * output, or test patches.
 *
 * Intentionally not in the Log: everything here. Bench results live beside
 * the copied session jsonl, not inside it.
 */

import type { RunHealth } from "@sage/core";
import type { ToolNetwork, ToolSandbox } from "@sage/tools";
import type { CommandIo } from "../io.js";
import type { BenchPolicy, ScorerApply, ScorerName } from "../options.js";

/**
 * Directory prefix of a bench run root under the real temp dir. The bash
 * sandbox denies reads of sibling roots with this prefix.
 */
export const BENCH_WORKSPACE_PREFIX = "sage-bench-";

/**
 * One task from the tasks file. `testPatch` is a unified diff applied after
 * the agent finishes; the agent never sees it.
 */
export type BenchTask = {
  id: string;
  /** `owner/name` on GitHub, or any URL with a scheme. */
  repo: string;
  baseCommit: string;
  problem: string;
  testCommand: string;
  /** Limit for setup, the agent, and the test command, each. */
  timeoutSec: number;
  setupCommand?: string;
  testPatch?: string;
};

/** Task outcome: tests passed, the agent changed nothing, or anything else. */
export type BenchStatus = "resolved" | "empty-patch" | "error";

/** Why a task ended as `error`. */
export type BenchErrorReason =
  | "clone-failed"
  | "clone-unclean"
  | "setup-failed"
  | "sage-failed"
  | "timeout"
  | "diff-failed"
  | "test-patch-failed"
  | "tests-failed";

/**
 * Outcome of one task. `reason` is a {@link BenchErrorReason} when the
 * runner wrote it; a hand-edited results file may hold any string.
 * `health` is `runHealth` of the copied session, or null when none was
 * copied or it could not be read.
 */
export type BenchTaskResult = {
  id: string;
  status: BenchStatus;
  reason?: string;
  health: RunHealth | null;
};

/** Contents of `results.json`. */
export type BenchResults = {
  policy: BenchPolicy;
  model: string;
  /** `--budget` for a compiler run; null when omitted or under passthrough. */
  budget: number | null;
  toolNetwork: ToolNetwork;
  maxTurns: number;
  scorerApply: ScorerApply;
  tasks: BenchTaskResult[];
};

/** Report rates. Each is null when its denominator is zero. */
export type BenchMetrics = {
  resolvedRate: number | null;
  emptyPatchRate: number | null;
  errorRate: number | null;
  tokensPerResolved: number | null;
  cacheHitRatio: number | null;
  expandRate: number | null;
};

/**
 * Settings for each child `sage -p`. Optional scorer fields are forwarded
 * only when the bench flag was written.
 */
export type SageRunOptions = {
  policy: BenchPolicy;
  model: string;
  provider?: string;
  env: NodeJS.ProcessEnv;
  budget: number | null;
  toolNetwork: ToolNetwork;
  /** `allowRoot` is the run root when `fsIsolate` is on. */
  sandbox: ToolSandbox;
  maxTurns: number;
  shadowScorer?: ScorerName;
  scorer?: ScorerName;
  scorerApply?: ScorerApply;
};

/** Outcome of the child `sage -p`. `sessionFile` is its newest jsonl. */
export type SageRun = {
  code: number;
  sessionFile?: string;
  timedOut?: boolean;
};

/**
 * Side effects of one task. Test seam: production uses git, the sage bin,
 * and `sh -c <testCommand>` (see `defaultDeps`).
 */
export type BenchDeps = {
  /** Throws `CloneUncleanError` or any error for a failed fetch. */
  prepare(task: BenchTask, cwd: string): Promise<void>;
  /** `opts.argv` is the full child argv, starting with the sage bin. */
  runSage(
    task: BenchTask,
    cwd: string,
    opts: SageRunOptions & { argv: string[] },
  ): Promise<SageRun>;
  /** True when `git status --porcelain` is empty. */
  diffEmpty(task: BenchTask, cwd: string): Promise<boolean>;
  runTests(
    task: BenchTask,
    cwd: string,
  ): Promise<{ code: number; timedOut?: boolean }>;
};

/** Process handles for the bench command. */
export type BenchIo = CommandIo & {
  env: NodeJS.ProcessEnv;
  /** Test seam. Production uses `defaultDeps()`. */
  deps?: BenchDeps;
};
