/**
 * bash tool: runs one shell command in the workspace, optionally inside an
 * OS sandbox that cuts the child's network and hides other bench
 * workspaces from it.
 *
 * Guarantees: the result starts with `[bash exit code N]`. Output over
 * `SPILL_LIMIT_BYTES` is written to `<cwd>/.sage/tool-output/<toolUseId>.txt`
 * and the model gets a head/tail preview. A sandbox the platform cannot
 * enforce fails with `ToolNetworkError` when the tool is created; there is
 * no fallback to an open network. A command the sandbox denies is not
 * rewritten; the child prints its own error. Without a sandbox the default
 * exec does not confine the filesystem and is meant for trusted
 * environments. Sandbox rules per platform: `createBash`.
 *
 * Bash cannot declare resource accesses statically, so `accesses()` always
 * returns [] and the tool never joins the read-only concurrency pool.
 *
 * Intentionally not in the Log: the child process, the abort signal, the
 * sandbox profile, the temp root, allowRoot, and the spilled file. The
 * preview the model sees is the tool result. With network none, execute
 * returns `details: { network: "none" }`; the kernel stores it on the user
 * message as `details.toolResults`, keyed by toolUseId, and it is not
 * model-visible. fsIsolate is not a details field.
 */

import { type ChildProcess, spawn, spawnSync } from "node:child_process";
import { realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { errorText, type HarnessTool } from "@sage/core";
import { asRecord, fail, ok, optionalInt, requiredString } from "./input.js";
import { TOOL_NAMES } from "./names.js";
import { SAGE_DIR_NAME } from "./path-safety.js";
import { spill } from "./spill.js";

/** Default `timeout_ms` of a bash call, in milliseconds. */
export const BASH_TIMEOUT_MS = 120_000;

/**
 * Exit code reported when the timeout killed the child, as GNU `timeout(1)`
 * reports it. The note `timed out after <ms>ms` is appended to stderr.
 */
const TIMEOUT_EXIT_CODE = 124;
/** Exit code reported when the child closed without one (killed by a signal). */
const SIGNALED_EXIT_CODE = 1;
/** How long the `unshare -n true` probe may run before it counts as missing. */
const UNSHARE_PROBE_TIMEOUT_MS = 2000;

/** Network policy of the bash child: `full` is unrestricted, `none` is cut off. */
export type ToolNetwork = "full" | "none";

const DEFAULT_BENCH_PREFIX = "sage-bench-";

/** Child env bench uses to pass the run root. Not a CLI flag. */
export const SANDBOX_ALLOW_ROOT_ENV = "SAGE_SANDBOX_ALLOW_ROOT";

/** Sandbox settings after defaults; see `resolveToolSandbox`. */
export type ToolSandbox = {
  network: ToolNetwork;
  /** Deny reads of other bench workspaces under the temp root (darwin). */
  fsIsolate: boolean;
  /** Directory tree bash may read. Omitted means the cwd realpath. */
  allowRoot?: string;
};

/**
 * Fills sandbox defaults: `network` is `full`, and `fsIsolate` is true
 * exactly when network is `none`. An empty `allowRoot` is dropped.
 */
export function resolveToolSandbox(sandbox?: {
  network?: ToolNetwork;
  fsIsolate?: boolean;
  allowRoot?: string;
}): ToolSandbox {
  const network = sandbox?.network ?? "full";
  const fsIsolate = sandbox?.fsIsolate ?? network === "none";
  const allowRoot = sandbox?.allowRoot;
  if (typeof allowRoot === "string" && allowRoot.length > 0) {
    return { network, fsIsolate, allowRoot };
  }
  return { network, fsIsolate };
}

/** What a `BashExec` receives besides the command string. */
export type BashExecOpts = {
  /** The workspace realpath; the child runs there. */
  cwd: string;
  signal?: AbortSignal;
  timeoutMs: number;
  /** The launch argv, already wrapped in the sandbox command when one applies. */
  argv: readonly string[];
  /** True when `command` runs through a shell instead of `argv`. */
  shell: boolean;
};

/**
 * Runs one bash call. A custom exec replaces the spawn and still receives
 * the sandbox-wrapped argv. It resolves with the exit code; a rejection
 * becomes an isError tool result.
 */
export type BashExec = (
  command: string,
  opts: BashExecOpts,
) => Promise<{ stdout: string; stderr: string; code: number }>;

/**
 * The requested sandbox cannot be enforced on this platform. Thrown when
 * the tools are created, never from execute. `exitCode` is the process exit
 * code the CLI uses for this startup configuration error.
 */
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

/**
 * Says whether this platform can enforce the sandbox. Network none follows
 * `toolNetworkSupport`. linux `unshare -n` cannot restrict files, so with
 * network none fsIsolate neither changes the argv nor fails the launch.
 * Filesystem isolation with network full is darwin-only.
 */
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

/**
 * Says whether this platform can cut the child's network. darwin always can
 * (`sandbox-exec`, localhost still allowed); linux can when `unshare -n`
 * works, probed on each call unless `unshareAvailable` is given; every
 * other platform cannot.
 */
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
      description: `Timeout in milliseconds. Defaults to ${BASH_TIMEOUT_MS}.`,
    },
  },
  required: ["command"],
};

/**
 * Creates the bash tool.
 *
 * `cwd` is the workspace path as the caller spelled it; the spill path the
 * model sees is built from it. `realCwd` is its realpath: the child runs
 * there, and it is the default sandbox allow root, because `sandbox-exec`
 * matches resolved paths (macOS `/var` is `/private/var`).
 *
 * Network full without fsIsolate runs the command in a shell. Otherwise
 * darwin wraps it in `sandbox-exec` (see `sandboxProfile`) and linux with
 * network none wraps it in `unshare -n`; any other combination throws
 * `ToolNetworkError` from execute's launch step, which becomes an isError
 * result. `tmpRoot` defaults to the realpath of `os.tmpdir()` and
 * `benchPrefix` to `sage-bench-`.
 */
export function createBash(
  cwd: string,
  realCwd: string,
  exec: BashExec,
  opts: {
    sandbox?: {
      network?: ToolNetwork;
      fsIsolate?: boolean;
      allowRoot?: string;
    };
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
      name: TOOL_NAMES.bash,
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
        const read = readBashInput(input);
        if (!read.ok) return fail(read.error);
        const launch = bashLaunch(read.command, sandbox, platform, {
          cwd: realCwd,
          tmpRoot,
          benchPrefix,
        });
        const result = await exec(read.command, {
          cwd: realCwd,
          signal: ctx.signal,
          timeoutMs: read.timeoutMs,
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

/** The command and timeout from the model's input, or why it is invalid. */
function readBashInput(
  input: unknown,
):
  | { ok: true; command: string; timeoutMs: number }
  | { ok: false; error: string } {
  const record = asRecord(input);
  if (!record) return { ok: false, error: "input must be an object" };
  const command = requiredString(record, "command");
  if (!command) return { ok: false, error: "command must be a string" };
  const timeoutArg = optionalInt(record, "timeout_ms");
  if (!timeoutArg.ok) {
    return { ok: false, error: "timeout_ms must be an integer" };
  }
  const timeoutMs = timeoutArg.value ?? BASH_TIMEOUT_MS;
  if (timeoutMs < 1) return { ok: false, error: "timeout_ms must be >= 1" };
  return { ok: true, command, timeoutMs };
}

/**
 * Spawns the command and collects stdout and stderr. A sandboxed launch
 * runs detached so a timeout or abort kills the whole process group with
 * SIGKILL. Spawn errors reject; every exit, including a kill, resolves.
 */
export function defaultExec(
  command: string,
  opts: BashExecOpts,
): Promise<{ stdout: string; stderr: string; code: number }> {
  return new Promise((resolve, reject) => {
    const child = spawnChild(command, opts);
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    let settled = false;
    const killer = armKill(child, opts);
    child.stdout?.on("data", (chunk: Buffer) => {
      stdout.push(chunk);
    });
    child.stderr?.on("data", (chunk: Buffer) => {
      stderr.push(chunk);
    });
    child.on("error", (error) => {
      if (settled) return;
      settled = true;
      killer.stop();
      reject(error);
    });
    child.on("close", (code) => {
      if (settled) return;
      settled = true;
      killer.stop();
      const timedOut = killer.timedOut();
      const errText = Buffer.concat(stderr).toString("utf8");
      const note = timedOut ? `\ntimed out after ${opts.timeoutMs}ms` : "";
      resolve({
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: `${errText}${note}`,
        code: timedOut ? TIMEOUT_EXIT_CODE : (code ?? SIGNALED_EXIT_CODE),
      });
    });
  });
}

/** A shell launch runs attached; an argv launch runs detached. */
function spawnChild(command: string, opts: BashExecOpts): ChildProcess {
  if (opts.shell) {
    return spawn(command, {
      cwd: opts.cwd,
      shell: true,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
  }
  const file = opts.argv[0] ?? command;
  return spawn(file, opts.argv.slice(1), {
    cwd: opts.cwd,
    shell: false,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
    detached: true,
  });
}

/**
 * Kills the child when `opts.timeoutMs` elapses or `opts.signal` aborts.
 * `timedOut` reports whether the timer fired; `stop` disarms both.
 */
function armKill(
  child: ChildProcess,
  opts: BashExecOpts,
): { timedOut: () => boolean; stop: () => void } {
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
  return {
    timedOut: () => timedOut,
    stop() {
      clearTimeout(timer);
      opts.signal?.removeEventListener("abort", onAbort);
    },
  };
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

/**
 * The darwin `sandbox-exec` profile. Network none denies network except
 * localhost. fsIsolate denies file reads matching
 * `^<tmpRoot>/<benchPrefix>` (regex metacharacters escaped), then allows
 * the allow root. Other files in the temp directory stay readable, and
 * paths outside the temp root fall under `(allow default)`.
 */
function sandboxProfile(
  sandbox: ToolSandbox,
  paths: { cwd: string; tmpRoot: string; benchPrefix: string },
): string {
  const rules = ["(version 1)", "(allow default)"];
  if (sandbox.network === "none") {
    rules.push("(deny network*)", '(allow network* (local ip "localhost:*"))');
  }
  if (sandbox.fsIsolate) {
    // Later rules win. Deny other bench roots, then allow allowRoot.
    // The default is the cwd. A wider root lets `cd` stat that parent.
    const allowed = sandbox.allowRoot
      ? realpathSync(sandbox.allowRoot)
      : paths.cwd;
    const pattern = `^${escapeSandboxRegex(paths.tmpRoot)}/${escapeSandboxRegex(paths.benchPrefix)}`;
    rules.push(
      `(deny file-read* (regex #"${pattern}"))`,
      `(allow file-read* (subpath ${JSON.stringify(allowed)}))`,
    );
  }
  return rules.join("");
}

function escapeSandboxRegex(value: string): string {
  return value.replace(/[\\^$.*+?()[\]{}|]/g, "\\$&");
}

function probeUnshare(): boolean {
  const result = spawnSync("unshare", ["-n", "true"], {
    timeout: UNSHARE_PROBE_TIMEOUT_MS,
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
  return path.join(cwd, SAGE_DIR_NAME, "tool-output", `${safe}.txt`);
}

function combine(stdout: string, stderr: string): string {
  if (stdout.length === 0) return stderr;
  if (stderr.length === 0) return stdout;
  const gap = stdout.endsWith("\n") ? "" : "\n";
  return `${stdout}${gap}${stderr}`;
}
