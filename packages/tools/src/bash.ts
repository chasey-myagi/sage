/**
 * bash tool. Runs one shell command in the workspace.
 *
 * The default exec is for trusted environments. It does not confine the
 * filesystem. `sandbox.network` defaults to "full" and runs the command in
 * a shell. "none" cuts off only that child: darwin wraps it in
 * `sandbox-exec` and still allows localhost; linux wraps it in `unshare -n`
 * when that command works. Other platforms, and linux without `unshare -n`,
 * throw ToolNetworkError when the tools are created. There is no fallback
 * to an open network. A custom exec replaces the spawn and still receives
 * the wrapped argv.
 *
 * `sandbox.fsIsolate` defaults to true when network is none. On darwin the
 * profile then denies file reads of other bench workspaces under the
 * realpath of os.tmpdir(): `(deny file-read* (regex #"^<tmpRoot>/<prefix>"))`.
 * Regex metacharacters in that path and prefix are escaped. `benchPrefix`
 * defaults to `sage-bench-`. The cwd realpath is allowed afterwards; later
 * rules win, so the deny is written first. macOS `/var` is stored as
 * `/private/var`. Ordinary files in the temp directory stay readable. linux
 * `unshare -n` cannot restrict files; fsIsolate does not change that argv
 * and does not fail a network-none launch. Filesystem isolation with
 * network full is darwin-only.
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
 * prints its own error. fsIsolate is not a details field.
 *
 * Intentionally not in the Log: the child process, the abort signal, the
 * sandbox profile, the temp root, and the spilled file. The preview the
 * model sees is the tool result.
 */

import { type ChildProcess, spawn, spawnSync } from "node:child_process";
import { realpathSync } from "node:fs";
import { tmpdir } from "node:os";
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

export type ToolNetwork = "full" | "none";

const DEFAULT_BENCH_PREFIX = "sage-bench-";

export type ToolSandbox = {
  network: ToolNetwork;
  fsIsolate: boolean;
};

export function resolveToolSandbox(sandbox?: {
  network?: ToolNetwork;
  fsIsolate?: boolean;
}): ToolSandbox {
  const network = sandbox?.network ?? "full";
  const fsIsolate = sandbox?.fsIsolate ?? network === "none";
  return { network, fsIsolate };
}

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

export function toolSandboxSupport(opts: {
  platform: NodeJS.Platform;
  network: ToolNetwork;
  fsIsolate: boolean;
  unshareAvailable?: boolean;
}): { ok: true } | { ok: false; message: string } {
  if (opts.network === "none") {
    return toolNetworkSupport({
      platform: opts.platform,
      ...(opts.unshareAvailable !== undefined
        ? { unshareAvailable: opts.unshareAvailable }
        : {}),
    });
  }
  if (opts.fsIsolate && opts.platform !== "darwin") {
    return {
      ok: false,
      message: `tool filesystem isolation is not supported on ${opts.platform}`,
    };
  }
  return { ok: true };
}

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
  opts: {
    sandbox?: { network?: ToolNetwork; fsIsolate?: boolean };
    platform?: NodeJS.Platform;
    tmpRoot?: string;
    benchPrefix?: string;
  } = {},
): HarnessTool {
  const sandbox = resolveToolSandbox(opts.sandbox);
  const platform = opts.platform ?? process.platform;
  const tmpRoot = opts.tmpRoot ?? realpathSync(tmpdir());
  const benchPrefix = opts.benchPrefix ?? DEFAULT_BENCH_PREFIX;
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
        const launch = bashLaunch(command, sandbox, platform, {
          cwd: execCwd,
          tmpRoot,
          benchPrefix,
        });
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
        if (sandbox.network !== "none") return base;
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
  sandbox: ToolSandbox,
  platform: NodeJS.Platform,
  paths: { cwd: string; tmpRoot: string; benchPrefix: string },
): { argv: string[]; shell: boolean } {
  if (sandbox.network === "full" && !sandbox.fsIsolate) {
    return { argv: [command], shell: true };
  }
  if (platform === "darwin") {
    return {
      argv: [
        "sandbox-exec",
        "-p",
        sandboxProfile(sandbox, paths),
        "sh",
        "-c",
        command,
      ],
      shell: false,
    };
  }
  if (platform === "linux" && sandbox.network === "none") {
    return {
      argv: ["unshare", "-n", "sh", "-c", command],
      shell: false,
    };
  }
  if (sandbox.fsIsolate) {
    throw new ToolNetworkError(
      `tool filesystem isolation is not supported on ${platform}`,
    );
  }
  throw new ToolNetworkError(UNSUPPORTED(platform));
}

function sandboxProfile(
  sandbox: ToolSandbox,
  paths: { cwd: string; tmpRoot: string; benchPrefix: string },
): string {
  const rules = ["(version 1)", "(allow default)"];
  if (sandbox.network === "none") {
    rules.push("(deny network*)", '(allow network* (local ip "localhost:*"))');
  }
  if (sandbox.fsIsolate) {
    // Later rules win. Deny sibling bench workspaces, then allow this cwd.
    const pattern = `^${escapeSandboxRegex(paths.tmpRoot)}/${escapeSandboxRegex(paths.benchPrefix)}`;
    rules.push(
      `(deny file-read* (regex #"${pattern}"))`,
      `(allow file-read* (subpath ${JSON.stringify(paths.cwd)}))`,
    );
  }
  return rules.join("");
}

function escapeSandboxRegex(value: string): string {
  return value.replace(/[\\^$.*+?()[\]{}|]/g, "\\$&");
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
