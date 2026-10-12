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

import { errorText, type HarnessTool } from "@sage/core";
import { pathAccess } from "./access.js";
import { globToRegExp } from "./glob.js";
import { asRecord, fail, ok, optionalString, requiredString } from "./input.js";
import { TOOL_NAMES } from "./names.js";
import { resolveInside, workspaceRelative } from "./path-safety.js";
import { LIST_MAX_ENTRIES } from "./truncate.js";
import { walkWorkspaceFiles } from "./walk.js";

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
        const found = walkWorkspaceFiles(root)
          .map((abs) => workspaceRelative(cwd, abs))
          .filter((rel) => matcher.test(rel))
          .sort();
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
