/**
 * First-party tools.
 *
 * Contract: createTools returns read, write, edit, bash, grep, find, and ls.
 * Each declares isReadOnly, isDestructive, isOpenWorld, and accesses().
 * `network` defaults to "full". "none" throws ToolNetworkError unless this
 * platform can enforce it (darwin, or linux with `unshare -n`). The kernel
 * does not import this package.
 *
 * Intentionally not in the Log: Access lists, exec injection, and spilled
 * files under `.sage/tool-output`. Model-visible text leaves only as tool
 * results, which the session logs. When network is none, bash details
 * `{ network: "none" }` ride the kernel's `details.toolResults` and are
 * not model-visible.
 */

import { realpathSync } from "node:fs";
import {
  type BashExec,
  createBash,
  defaultExec,
  type ToolNetwork,
  ToolNetworkError,
  toolNetworkSupport,
} from "./bash.js";
import { createEdit } from "./edit.js";
import { createFind } from "./find.js";
import { createGrep } from "./grep.js";
import { createLs } from "./ls.js";
import { createRead } from "./read.js";
import { createWrite } from "./write.js";

export const PACKAGE = "@sage/tools" as const;

export type { BashExec, ToolNetwork };
export { ToolNetworkError, toolNetworkSupport };

export function createTools(opts: {
  cwd: string;
  exec?: BashExec;
  network?: ToolNetwork;
  platform?: NodeJS.Platform;
  unshareAvailable?: boolean;
}) {
  const network = opts.network ?? "full";
  const platform = opts.platform ?? process.platform;
  if (network === "none") {
    const support = toolNetworkSupport({
      platform,
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
    createBash(cwd, execCwd, exec, { network, platform }),
    createGrep(cwd),
    createFind(cwd),
    createLs(cwd),
  ];
}
