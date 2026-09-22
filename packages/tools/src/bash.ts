/**
 * bash tool. Runs one shell command in the workspace.
 *
 * The default exec is for trusted environments. It does not confine the
 * filesystem. `network: "full"` (the default) runs the command in a shell.
 * `network: "none"` cuts off only that child: darwin wraps it in
 * `sandbox-exec` and still allows localhost; linux wraps it in `unshare -n`
 * when that command works. Other platforms, and linux without `unshare -n`,
 * throw ToolNetworkError when the tools are created. There is no fallback
 * to an open network. A custom exec replaces the spawn and still receives
 * the wrapped argv.
 *
 * Bash cannot declare resource accesses statically, so accesses() always
 * returns [] and the tool never joins the read-only concurrency pool.
 *
 * Contract: output over 30000 bytes is spilled under
 * `<cwd>/.sage/tool-output/<toolUseId>.txt`. The result starts with
 * `[bash exit code N]`. timeout_ms defaults to 120000. When network is
 * none, execute returns `details: { network: "none" }`. The kernel stores
 * that on the user message as `details.toolResults`, keyed by toolUseId.
 * It is not model-visible. A denied command is not rewritten; the child
 * prints its own error.
 *
 * Intentionally not in the Log: the child process, the abort signal, the
 * sandbox profile, and the spilled file. The preview the model sees is the
 * tool result.
 */

import { type ChildProcess, spawn, spawnSync } from "node:child_process";
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

export const BASH_TIMEOUT_MS = 120_000;

const SANDBOX_PROFILE =
  '(version 1)(allow default)(deny network*)(allow network* (local ip "localhost:*"))';

export type ToolNetwork = "full" | "none";

export type BashExecOpts = {
  cwd: string;
  signal?: AbortSignal;
  timeoutMs: number;
  argv: readonly string[];
  shell: boolean;
};

export type BashExec = (
  command: string,
  opts: BashExecOpts,
) => Promise<{ stdout: string; stderr: string; code: number }>;

export class ToolNetworkError extends Error {
  readonly exitCode = 2;
  constructor(message: string) {
    super(message);
    this.name = "ToolNetworkError";
  }
}

const UNSUPPORTED = (platform: string) =>
  `tool network none is not supported on ${platform}`;

const UNSHARE_MISSING =
  "tool network none requires unshare -n, which is not available";

export function toolNetworkSupport(opts: {
  platform: NodeJS.Platform;
  unshareAvailable?: boolean;
}): { ok: true } | { ok: false; message: string } {
  if (opts.platform === "darwin") return { ok: true };
  if (opts.platform === "linux") {
    const available =
      opts.unshareAvailable !== undefined
        ? opts.unshareAvailable
        : probeUnshare();
    if (!available) return { ok: false, message: UNSHARE_MISSING };
    return { ok: true };
  }
  return { ok: false, message: UNSUPPORTED(opts.platform) };
}

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
  opts: { network?: ToolNetwork; platform?: NodeJS.Platform } = {},
): HarnessTool {
  const network = opts.network ?? "full";
  const platform = opts.platform ?? process.platform;
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
        const launch = bashLaunch(command, network, platform);
        const result = await exec(command, {
          cwd: execCwd,
          signal: ctx.signal,
          timeoutMs,
          argv: launch.argv,
          shell: launch.shell,
        });
        const combined = combine(result.stdout, result.stderr);
        const body = spill(combined, spillPath(cwd, ctx.toolUseId)).text;
        const content = `[bash exit code ${result.code}]\n${body}`;
        const base = result.code === 0 ? ok(content) : fail(content);
        if (network !== "none") return base;
        return { ...base, details: { network: "none" } };
      } catch (error) {
        return fail(errorText(error));
      }
    },
  };
}

export function defaultExec(
  command: string,
  opts: BashExecOpts,
): Promise<{ stdout: string; stderr: string; code: number }> {
  return new Promise((resolve, reject) => {
    const file = opts.argv[0] ?? command;
    const child = opts.shell
      ? spawn(command, {
          cwd: opts.cwd,
          shell: true,
          stdio: ["ignore", "pipe", "pipe"],
          windowsHide: true,
        })
      : spawn(file, opts.argv.slice(1), {
          cwd: opts.cwd,
          shell: false,
          stdio: ["ignore", "pipe", "pipe"],
          windowsHide: true,
          detached: true,
        });
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    let settled = false;
    let timedOut = false;
    const kill = () => killChild(child, !opts.shell);
    const timer = setTimeout(() => {
      timedOut = true;
      kill();
    }, opts.timeoutMs);
    const onAbort = () => {
      kill();
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

function bashLaunch(
  command: string,
  network: ToolNetwork,
  platform: NodeJS.Platform,
): { argv: string[]; shell: boolean } {
  if (network === "full") return { argv: [command], shell: true };
  if (platform === "darwin") {
    return {
      argv: ["sandbox-exec", "-p", SANDBOX_PROFILE, "sh", "-c", command],
      shell: false,
    };
  }
  if (platform === "linux") {
    return {
      argv: ["unshare", "-n", "sh", "-c", command],
      shell: false,
    };
  }
  throw new ToolNetworkError(UNSUPPORTED(platform));
}

function probeUnshare(): boolean {
  const result = spawnSync("unshare", ["-n", "true"], {
    timeout: 2000,
    stdio: "ignore",
  });
  return result.status === 0;
}

function killChild(child: ChildProcess, group: boolean): void {
  if (group && child.pid !== undefined && child.pid > 0) {
    try {
      process.kill(-child.pid, "SIGKILL");
      return;
    } catch {
      // The child has already exited, or it is not a group leader.
    }
  }
  child.kill("SIGKILL");
}

function spillPath(cwd: string, toolUseId: string): string {
  const safe = toolUseId.replace(/[^A-Za-z0-9._-]/g, "_");
  return path.join(cwd, ".sage", "tool-output", `${safe}.txt`);
}

function combine(stdout: string, stderr: string): string {
  if (stdout.length === 0) return stderr;
  if (stderr.length === 0) return stdout;
  const gap = stdout.endsWith("\n") ? "" : "\n";
  return `${stdout}${gap}${stderr}`;
}
