/**
 * bash tool. Runs one shell command in the workspace.
 *
 * The default exec is for trusted environments only. Production must inject
 * a sandboxed exec through this seam. This package does not implement a sandbox.
 *
 * Bash cannot declare resource accesses statically, so accesses() always
 * returns [] and the tool never joins the read-only concurrency pool.
 *
 * Contract: output over 30000 bytes is spilled under
 * `<cwd>/.sage/tool-output/<toolUseId>.txt`. The result starts with
 * `[bash exit code N]`. timeout_ms defaults to 120000.
 *
 * Intentionally not in the Log: the child process, the abort signal, and the
 * spilled file. The preview the model sees is the tool result.
 */

import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import path from "node:path";
import type { HarnessTool } from "@sage/core";
import {
  asRecord,
  errorText,
  fail,
  ok,
  optionalInt,
  requiredString,
} from "./input.js";
import { spill } from "./spill.js";
import { currentToolUseId } from "./tool-use.js";

export const BASH_TIMEOUT_MS = 120_000;

export type BashExec = (
  command: string,
  opts: { cwd: string; signal?: AbortSignal; timeoutMs: number },
) => Promise<{ stdout: string; stderr: string; code: number }>;

const inputSchema = {
  type: "object",
  properties: {
    command: {
      type: "string",
      description: "Shell command to run in the workspace.",
    },
    timeout_ms: {
      type: "integer",
      minimum: 1,
      description: "Timeout in milliseconds. Defaults to 120000.",
    },
  },
  required: ["command"],
};

export function createBash(
  cwd: string,
  execCwd: string,
  exec: BashExec,
): HarnessTool {
  return {
    def: {
      name: "bash",
      description:
        "Run a shell command in the workspace. Use it when no dedicated tool can do the step.",
      inputSchema,
    },
    isReadOnly: false,
    isDestructive: true,
    isOpenWorld: true,
    accesses() {
      return [];
    },
    async execute(input, ctx) {
      try {
        const record = asRecord(input);
        if (!record) return fail("input must be an object");
        const command = requiredString(record, "command");
        if (!command) return fail("command must be a string");
        const timeoutArg = optionalInt(record, "timeout_ms");
        if (!timeoutArg.ok) return fail("timeout_ms must be an integer");
        const timeoutMs = timeoutArg.value ?? BASH_TIMEOUT_MS;
        if (timeoutMs < 1) return fail("timeout_ms must be >= 1");
        const result = await exec(command, {
          cwd: execCwd,
          signal: ctx.signal,
          timeoutMs,
        });
        const combined = combine(result.stdout, result.stderr);
        const body = spill(combined, spillPath(cwd)).text;
        const content = `[bash exit code ${result.code}]\n${body}`;
        return result.code === 0 ? ok(content) : fail(content);
      } catch (error) {
        return fail(errorText(error));
      }
    },
  };
}

export function defaultExec(
  command: string,
  opts: { cwd: string; signal?: AbortSignal; timeoutMs: number },
): Promise<{ stdout: string; stderr: string; code: number }> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, {
      cwd: opts.cwd,
      shell: true,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    let settled = false;
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill("SIGKILL");
    }, opts.timeoutMs);
    const onAbort = () => {
      child.kill("SIGKILL");
    };
    if (opts.signal) {
      if (opts.signal.aborted) onAbort();
      else opts.signal.addEventListener("abort", onAbort, { once: true });
    }
    const stop = () => {
      clearTimeout(timer);
      opts.signal?.removeEventListener("abort", onAbort);
    };
    child.stdout?.on("data", (chunk: Buffer) => {
      stdout.push(chunk);
    });
    child.stderr?.on("data", (chunk: Buffer) => {
      stderr.push(chunk);
    });
    child.on("error", (error) => {
      if (settled) return;
      settled = true;
      stop();
      reject(error);
    });
    child.on("close", (code) => {
      if (settled) return;
      settled = true;
      stop();
      const errText = Buffer.concat(stderr).toString("utf8");
      const note = timedOut ? `\ntimed out after ${opts.timeoutMs}ms` : "";
      resolve({
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: `${errText}${note}`,
        code: timedOut ? 124 : (code ?? 1),
      });
    });
  });
}

function spillPath(cwd: string): string {
  const raw = currentToolUseId() ?? "";
  const safe = raw.replace(/[^A-Za-z0-9._-]/g, "_");
  const id = safe.length > 0 ? safe : randomUUID();
  return path.join(cwd, ".sage", "tool-output", `${id}.txt`);
}

function combine(stdout: string, stderr: string): string {
  if (stdout.length === 0) return stderr;
  if (stderr.length === 0) return stdout;
  const gap = stdout.endsWith("\n") ? "" : "\n";
  return `${stdout}${gap}${stderr}`;
}
