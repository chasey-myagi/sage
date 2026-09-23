/**
 * Child-process runner for bench: git, `sh -c`, and the sage bin.
 *
 * Contract: resolves with the exit code and stdout once the child closes;
 * rejects only when the child cannot be spawned. stderr is drained and
 * dropped. A child killed by `timeoutMs` resolves with `timedOut: true`
 * and code 1.
 *
 * Intentionally not in the Log: commands, their output, and exit codes.
 */

import { spawn } from "node:child_process";

/** Exit status and captured stdout of one child process. */
export type ExecResult = { code: number; timedOut: boolean; stdout: string };

/** Runs `command args` without a shell. `env` defaults to `process.env`. */
export function exec(
  command: string,
  args: string[],
  opts: { cwd?: string; env?: NodeJS.ProcessEnv; timeoutMs?: number } = {},
): Promise<ExecResult> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: opts.cwd,
      env: opts.env ?? process.env,
      timeout: opts.timeoutMs,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      stdout += chunk;
    });
    child.stderr.resume();
    child.on("error", reject);
    child.on("close", (code, signal) => {
      resolve({
        code: code ?? 1,
        timedOut: code === null && signal !== null,
        stdout,
      });
    });
  });
}
