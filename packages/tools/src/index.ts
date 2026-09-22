/**
 * First-party tools.
 *
 * Contract: createTools returns read, write, edit, bash, grep, find, and ls.
 * Each declares isReadOnly, isDestructive, isOpenWorld, and accesses(). The
 * kernel does not import this package.
 *
 * Intentionally not in the Log: Access lists, exec injection, and spilled
 * files under `.sage/tool-output`. Model-visible text leaves only as tool
 * results, which the session logs.
 */

import { realpathSync } from "node:fs";
import { type BashExec, createBash, defaultExec } from "./bash.js";
import { createEdit } from "./edit.js";
import { createFind } from "./find.js";
import { createGrep } from "./grep.js";
import { createLs } from "./ls.js";
import { createRead } from "./read.js";
import { createWrite } from "./write.js";

export const PACKAGE = "@sage/tools" as const;

export { withToolUseId } from "./tool-use.js";
export type { BashExec };

export function createTools(opts: { cwd: string; exec?: BashExec }) {
  const cwd = opts.cwd;
  const execCwd = realpathSync(cwd);
  const exec = opts.exec ?? defaultExec;
  return [
    createRead(cwd),
    createWrite(cwd),
    createEdit(cwd),
    createBash(cwd, execCwd, exec),
    createGrep(cwd),
    createFind(cwd),
    createLs(cwd),
  ];
}
