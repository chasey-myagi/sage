/**
 * First-party tools.
 *
 * Contract: createTools returns read, write, edit, bash, grep, find, and ls.
 * Each declares isReadOnly, isDestructive, isOpenWorld, and accesses().
 * `sandbox.network` defaults to "full". "none" throws ToolNetworkError
 * unless this platform can enforce it (darwin, or linux with `unshare -n`).
 * `sandbox.fsIsolate` defaults to true when network is none. On darwin that
 * denies reads of other bench workspaces under the real temp root. `tmpRoot`
 * and `benchPrefix` are options; the prefix defaults to `sage-bench-`. linux
 * `unshare -n` cannot restrict files. A path outside the workspace returns
 * an isError tool result and does not throw from execute or accesses. The
 * kernel does not import this package.
 *
 * Intentionally not in the Log: Access lists, exec injection, the sandbox
 * profile, and spilled files under `.sage/tool-output`. Model-visible text
 * leaves only as tool results, which the session logs. When network is
 * none, bash details `{ network: "none" }` ride the kernel's
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
import { createRead } from "./read.js";
import { createWrite } from "./write.js";

export const PACKAGE = "@sage/tools" as const;

export type { BashExec, ToolNetwork, ToolSandbox };
export {
  resolveToolSandbox,
  ToolNetworkError,
  toolNetworkSupport,
  toolSandboxSupport,
};

export function createTools(opts: {
  cwd: string;
  exec?: BashExec;
  sandbox?: { network?: ToolNetwork; fsIsolate?: boolean };
  platform?: NodeJS.Platform;
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
  const execCwd = realpathSync(cwd);
  const exec = opts.exec ?? defaultExec;
  return [
    createRead(cwd),
    createWrite(cwd),
    createEdit(cwd),
    createBash(cwd, execCwd, exec, {
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
