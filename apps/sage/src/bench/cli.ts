/**
 * `sage bench`: offline benchmark runner and report. Not a CI gate.
 *
 * Contract: dispatches `run` and `report`; any other subcommand prints the
 * bench usage and exits 1. Flag and result semantics are in
 * `apps/sage/README.md`.
 *
 * Intentionally not in the Log: argv, API keys, clone directories, setup
 * commands, test patches, test output, `SAGE_SANDBOX_ALLOW_ROOT`, and the
 * report. Each child `sage -p` writes its own session jsonl, which the
 * runner copies before the test patch is applied.
 */

import { benchUsage } from "./args.js";
import { runBenchReport } from "./report.js";
import { runBenchRun } from "./run.js";
import type { BenchIo } from "./types.js";

/** Runs `sage bench <argv>` and returns the process exit code. */
export async function runBenchCli(
  argv: string[],
  io: BenchIo,
): Promise<number> {
  const [command, ...rest] = argv;
  if (command === "run") return runBenchRun(rest, io);
  if (command === "report") return runBenchReport(rest, io);
  io.stderr.write(`${benchUsage()}\n`);
  return 1;
}
