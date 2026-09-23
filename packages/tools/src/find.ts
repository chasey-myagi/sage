/**
 * find tool. Lists workspace files matching a glob.
 *
 * Contract: paths are relative to the workspace, one per line, sorted.
 * Symlinks are not followed. `.git`, `.sage`, and `node_modules` are skipped.
 *
 * Intentionally not in the Log: paths that did not match. The list is the
 * tool result the session logs.
 */

import { readdirSync, statSync } from "node:fs";
import path from "node:path";
import type { HarnessTool } from "@sage/core";
import { pathAccess } from "./access.js";
import { globToRegExp } from "./glob.js";
import {
  asRecord,
  errorText,
  fail,
  ok,
  optionalString,
  requiredString,
} from "./input.js";
import { resolveInside, workspaceRelative } from "./path-safety.js";

const LIST_MAX = 500;
const SKIP_DIRS = new Set([".git", ".sage", "node_modules"]);

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

export function createFind(cwd: string): HarnessTool {
  return {
    def: {
      name: "find",
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
        const shown = found.slice(0, LIST_MAX);
        const lines = [...shown];
        if (found.length > LIST_MAX) {
          lines.push(
            `[truncated: showing ${LIST_MAX} of ${found.length} entries]`,
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
      if (SKIP_DIRS.has(entry.name)) continue;
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
