/**
 * find tool. Lists workspace files matching a glob.
 *
 * Contract: paths are relative to the workspace, one per line, sorted.
 * Symlinks are not followed. `SKIPPED_DIR_NAMES` (`.git`, `.sage`,
 * `node_modules`) are skipped.
 *
 * Intentionally not in the Log: paths that did not match. The list is the
 * tool result the session logs.
 */

import { readdirSync, statSync } from "node:fs";
import path from "node:path";
import { errorText, type HarnessTool } from "@sage/core";
import { pathAccess } from "./access.js";
import { globToRegExp } from "./glob.js";
import { asRecord, fail, ok, optionalString, requiredString } from "./input.js";
import { TOOL_NAMES } from "./names.js";
import {
  resolveInside,
  SKIPPED_DIR_NAMES,
  workspaceRelative,
} from "./path-safety.js";
import { LIST_MAX_ENTRIES } from "./truncate.js";

const inputSchema = {
  type: "object",
  properties: {
    glob: {
      type: "string",
      description: "Glob to match, such as **/*.ts.",
    },
    path: {
      type: "string",
      description: "Directory to search. Defaults to the workspace.",
    },
  },
  required: ["glob"],
};

/**
 * Creates the find tool bound to `cwd`. No match returns `No matches.`;
 * more than `LIST_MAX_ENTRIES` paths end with a `[truncated: ...]` line.
 */
export function createFind(cwd: string): HarnessTool {
  return {
    def: {
      name: TOOL_NAMES.find,
      description:
        "Find files by glob in the workspace. Use it to locate files before reading them.",
      inputSchema,
    },
    isReadOnly: true,
    isDestructive: false,
    isOpenWorld: false,
    accesses(input) {
      return pathAccess(cwd, input, "search", true, ".");
    },
    async execute(input) {
      try {
        const record = asRecord(input);
        if (!record) return fail("input must be an object");
        const glob = requiredString(record, "glob");
        if (!glob) return fail("glob must be a string");
        const pathArg = optionalString(record, "path");
        if (!pathArg.ok) return fail("path must be a string");
        const resolved = resolveInside(cwd, pathArg.value ?? ".");
        if (!resolved.ok) return resolved.result;
        const root = resolved.path;
        const matcher = globToRegExp(glob);
        const found = walk(cwd, root, matcher);
        if (found.length === 0) return ok("No matches.");
        const shown = found.slice(0, LIST_MAX_ENTRIES);
        const lines = [...shown];
        if (found.length > LIST_MAX_ENTRIES) {
          lines.push(
            `[truncated: showing ${LIST_MAX_ENTRIES} of ${found.length} entries]`,
          );
        }
        return ok(lines.join("\n"));
      } catch (error) {
        return fail(errorText(error));
      }
    },
  };
}

function walk(cwd: string, root: string, matcher: RegExp): string[] {
  const found: string[] = [];
  const stack = [root];
  while (stack.length > 0) {
    const current = stack.pop();
    if (!current) break;
    let entries: {
      name: string;
      isSymbolicLink(): boolean;
      isDirectory(): boolean;
      isFile(): boolean;
    }[];
    try {
      if (statSync(current).isFile()) {
        const rel = workspaceRelative(cwd, current);
        if (matcher.test(rel)) found.push(rel);
        continue;
      }
      entries = readdirSync(current, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      if (SKIPPED_DIR_NAMES.has(entry.name)) continue;
      const abs = path.join(current, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) {
        stack.push(abs);
        continue;
      }
      if (!entry.isFile()) continue;
      const rel = workspaceRelative(cwd, abs);
      if (matcher.test(rel)) found.push(rel);
    }
  }
  found.sort();
  return found;
}
