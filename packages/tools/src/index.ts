/**
 * First-party tools: read, write, edit, bash, grep, find, and ls.
 *
 * Guarantees: each tool declares isReadOnly, isDestructive, isOpenWorld,
 * and accesses(). A path outside the workspace returns an isError tool
 * result and does not throw from execute or accesses. The kernel does not
 * import this package.
 *
 * Intentionally not in the Log: Access lists, exec injection, the sandbox
 * profile, allowRoot, and spilled files under `.sage/tool-output`.
 * Model-visible text leaves only as tool results, which the session logs.
 * With network none, bash details `{ network: "none" }` ride the kernel's
 * `details.toolResults` and are not model-visible. fsIsolate is not a
 * details field.
 */

import { realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import {
  type BashExec,
  createBash,
  defaultExec,
  resolveToolSandbox,
  SANDBOX_ALLOW_ROOT_ENV,
  type ToolNetwork,
  ToolNetworkError,
  type ToolSandbox,
  toolNetworkSupport,
  toolSandboxSupport,
} from "./bash.js";
import { createEdit } from "./edit.js";
import { createFind } from "./find.js";
import { createGrep } from "./grep.js";
import { createLs } from "./ls.js";
import { TOOL_NAMES } from "./names.js";
import { createRead } from "./read.js";
import { createWrite } from "./write.js";

/** The package name. */
export const PACKAGE = "@sage/tools" as const;

export type { BashExec, ToolNetwork, ToolSandbox };
export {
  resolveToolSandbox,
  SANDBOX_ALLOW_ROOT_ENV,
  TOOL_NAMES,
  ToolNetworkError,
  toolNetworkSupport,
  toolSandboxSupport,
};

/**
 * Creates the seven tools bound to `cwd`.
 *
 * `sandbox.network` defaults to "full"; "none" needs darwin or linux with
 * `unshare -n`. `sandbox.fsIsolate` defaults to true when network is none;
 * on darwin it denies reads of other bench workspaces under the real temp
 * root, then allows `sandbox.allowRoot` (default: the cwd realpath). linux
 * `unshare -n` cannot restrict files. Throws `ToolNetworkError` when this
 * platform cannot enforce the sandbox. Details: the package README.
 */
export function createTools(opts: {
  cwd: string;
  /** Test seam. Production uses `defaultExec`, which spawns the child. */
  exec?: BashExec;
  sandbox?: {
    network?: ToolNetwork;
    fsIsolate?: boolean;
    allowRoot?: string;
  };
  /** Defaults to `process.platform`. */
  platform?: NodeJS.Platform;
  /** Defaults to probing `unshare -n true`. */
  unshareAvailable?: boolean;
  /** Realpath of the temp root. Defaults to realpath(os.tmpdir()). */
  tmpRoot?: string;
  /** Bench workspace directory prefix. Defaults to `sage-bench-`. */
  benchPrefix?: string;
}) {
  const sandbox = resolveToolSandbox(opts.sandbox);
  const platform = opts.platform ?? process.platform;
  if (sandbox.network === "none" || sandbox.fsIsolate) {
    const support = toolSandboxSupport({
      platform,
      network: sandbox.network,
      fsIsolate: sandbox.fsIsolate,
      ...(opts.unshareAvailable !== undefined
        ? { unshareAvailable: opts.unshareAvailable }
        : {}),
    });
    if (!support.ok) throw new ToolNetworkError(support.message);
  }
  const cwd = opts.cwd;
  const realCwd = realpathSync(cwd);
  const exec = opts.exec ?? defaultExec;
  return [
    createRead(cwd),
    createWrite(cwd),
    createEdit(cwd),
    createBash(cwd, realCwd, exec, {
      sandbox,
      platform,
      tmpRoot: opts.tmpRoot ?? realpathSync(tmpdir()),
      ...(opts.benchPrefix !== undefined
        ? { benchPrefix: opts.benchPrefix }
        : {}),
    }),
    createGrep(cwd),
    createFind(cwd),
    createLs(cwd),
  ];
}
