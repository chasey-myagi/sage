/**
 * CLI entry: dispatches `sage stats`, `sage bench`, and the one-shot
 * `sage [-p] <task>`.
 *
 * Contract: `runCli` never throws for bad input; it writes to the given
 * streams and returns the process exit code. Each subcommand owns its
 * parsing, output, and exit codes (see `apps/sage/README.md`). This module
 * is the bundle entry; `bin/sage.mjs` calls `runCli` and exits.
 *
 * Intentionally not in the Log: argv and the environment. Only the
 * one-shot run writes a session; see `print.ts`.
 */

import { runBenchCli } from "./bench/cli.js";
import type { CliIo } from "./io.js";
import { runPrint } from "./print.js";
import { runStats } from "./stats.js";

/**
 * Runs the CLI for `argv` (without the node and script paths). Fields left
 * out of `io` come from the real process.
 */
export async function runCli(
  argv: string[],
  io: Partial<CliIo> = {},
): Promise<number> {
  const resolved: CliIo = {
    ...io,
    stdout: io.stdout ?? process.stdout,
    stderr: io.stderr ?? process.stderr,
    cwd: io.cwd ?? process.cwd(),
    env: io.env ?? process.env,
  };
  const [command, ...rest] = argv;
  if (command === "stats") return runStats(rest, resolved);
  if (command === "bench") {
    return runBenchCli(rest, {
      ...resolved,
      ...(io.benchDeps ? { deps: io.benchDeps } : {}),
    });
  }
  return runPrint(argv, resolved);
}
