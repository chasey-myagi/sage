/**
 * Types shared by the bench runner, its report, and its test seams.
 *
 * Contract: `BenchResults` is the shape of `<out>/results.json`. It holds
 * ids, statuses, reasons, provider error classes, and health numbers, never
 * problem text, error messages, test output, or test patches.
 *
 * Intentionally not in the Log: everything here. Bench results live beside
 * the copied session jsonl, not inside it.
 */

import type { RunHealth } from "@sage/core";
import type { ToolNetwork, ToolSandbox } from "@sage/tools";
import type { CommandIo } from "../command-io.js";
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

/** Every reason the runner writes on an `error` row. */
export const BENCH_ERROR_REASONS = [
  "clone-failed",
  "clone-unclean",
  "setup-failed",
  "sage-failed",
  "session-error",
  "session-aborted",
  "timeout",
  "diff-failed",
  "test-patch-failed",
  "tests-failed",
] as const;

/** Why a task ended as `error`. One of {@link BENCH_ERROR_REASONS}. */
export type BenchErrorReason = (typeof BENCH_ERROR_REASONS)[number];

/**
 * Classification of the provider failure that ended a run, copied from the
 * last assistant message's `details.finish.error`. `status` is the HTTP
 * status, null when the call failed below HTTP or inside a 2xx stream;
 * `transport` mirrors `FinishError.transport`; `event` mirrors
 * `FinishError.event`, null unless the server ended the stream with an
 * error event.
 */
export type BenchProviderError = {
  status: number | null;
  transport: boolean;
  event: BenchStreamErrorEvent | null;
};

/**
 * The structured fields of an in-stream error event, each null when the
 * event did not carry it. `status` is the event's own status field, not
 * the HTTP status. The event's message is never copied.
 */
export type BenchStreamErrorEvent = {
  transient: boolean;
  type: string | null;
  code: string | number | null;
  status: number | null;
};

/**
 * Outcome of one task. `reason` is a {@link BenchErrorReason} when the
 * runner wrote it; a hand-edited results file may hold any string.
 * `providerError` is present only on `session-error` rows: the provider
 * failure that ended the run, or null when the run ended on a harness error
 * (a seam assertion, a missing adapter) rather than a provider finish.
 * `health` is `runHealth` of the copied session, or null when none was
 * copied or it could not be read. `stoppedEarly` is true when the agent
 * ended its own run after an edit it never followed with bash and the diff
 * is not empty. Null when the session or diff is unavailable, or the child
 * timed out before the diff was checked. This is a tool-order heuristic,
 * not evidence that bash actually verified the edit.
 */
export type BenchTaskResult = {
  id: string;
  status: BenchStatus;
  reason?: string;
  providerError?: BenchProviderError | null;
  health: RunHealth | null;
  stoppedEarly: boolean | null;
};

/** Contents of `results.json`. */
export type BenchResults = {
  policy: BenchPolicy;
  model: string;
  /**
   * `--provider`, or the auto-selected preset when omitted. Null only when
   * read back from a results file without it.
   */
  provider: string | null;
  /** `--budget` for a compiler run; null when omitted or under passthrough. */
  budget: number | null;
  /** `--base-url`; null when omitted. */
  baseUrl: string | null;
  /** `--api-key-env`, the variable name only; null when omitted. */
  apiKeyEnv: string | null;
  toolNetwork: ToolNetwork;
  maxTurns: number;
  scorerApply: ScorerApply;
  tasks: BenchTaskResult[];
};

/** Report rates. Each is null when its denominator is zero. */
export type BenchMetrics = {
  resolvedRate: number | null;
  emptyPatchRate: number | null;
  /**
   * Every `error` row over all tasks, faults included; `errorRate` minus
   * `faultRate` is the share the agent itself failed.
   */
  errorRate: number | null;
  /** Rows `isFaultRow` accepts over all tasks. */
  faultRate: number | null;
  tokensPerResolved: number | null;
  cacheHitRatio: number | null;
  expandRate: number | null;
  /** Tasks with `stoppedEarly === true` over all tasks. */
  stoppedEarlyRate: number | null;
};

/**
 * Settings for each child `sage -p`. Optional provider and scorer fields
 * are forwarded only when the bench flag was written.
 */
export type SageRunOptions = {
  policy: BenchPolicy;
  model: string;
  provider?: string;
  baseUrl?: string;
  apiKeyEnv?: string;
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
  /** True when `git status --porcelain` is empty, ignoring sage's `.sage` runtime directory. */
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
