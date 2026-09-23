/**
 * CLI entry: dispatches `sage stats`, `sage bench`, and the one-shot
 * `sage [-p] <task>`.
 *
 * Contract: `runCli` never throws for bad input; it writes to the given
 * streams and returns the process exit code. Each subcommand owns its
 * parsing, output, and exit codes (see `apps/sage/README.md`). Run
 * directly, this module exits with that code.
 *
 * Intentionally not in the Log: argv and the environment. Only the
 * one-shot run writes a session; see `print.ts`.
 */

import path from "node:path";
import { pathToFileURL } from "node:url";
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

function isDirectRun(): boolean {
  const entry = process.argv[1];
  if (!entry) return false;
  return import.meta.url === pathToFileURL(path.resolve(entry)).href;
}

if (isDirectRun()) {
  runCli(process.argv.slice(2))
    .then((code) => {
      process.exit(code);
    })
    .catch((error: unknown) => {
      const message = error instanceof Error ? error.message : String(error);
      process.stderr.write(`${message}\n`);
      process.exit(1);
    });
}
