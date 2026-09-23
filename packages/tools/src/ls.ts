/**
 * ls tool. Lists the names in one workspace directory.
 *
 * Contract: directories end with `/`, symlinks end with `@` and are not
 * followed. The default path is the workspace root.
 *
 * Intentionally not in the Log: entries that were truncated past the cap.
 * The listing is the tool result the session logs.
 */

import { readdirSync, statSync } from "node:fs";
import type { HarnessTool } from "@sage/core";
import { pathAccess } from "./access.js";
import { asRecord, errorText, fail, ok, optionalString } from "./input.js";
import { resolveInside } from "./path-safety.js";
import { LIST_MAX_ENTRIES } from "./truncate.js";

const inputSchema = {
  type: "object",
  properties: {
    path: {
      type: "string",
      description: "Directory to list. Defaults to the workspace.",
    },
  },
};

/**
 * Creates the ls tool bound to `cwd`. Names are sorted; more than
 * `LIST_MAX_ENTRIES` end with a `[truncated: ...]` line. A path that is not
 * a directory is an isError result.
 */
export function createLs(cwd: string): HarnessTool {
  return {
    def: {
      name: "ls",
      description:
        "List entries in a workspace directory. Use it to see what a folder contains.",
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
        const record = asRecord(input) ?? {};
        const pathArg = optionalString(record, "path");
        if (!pathArg.ok) return fail("path must be a string");
        const resolved = resolveInside(cwd, pathArg.value ?? ".");
        if (!resolved.ok) return resolved.result;
        const abs = resolved.path;
        const stat = statSync(abs);
        if (!stat.isDirectory())
          return fail(`not a directory: ${pathArg.value ?? "."}`);
        const names = readdirSync(abs, { withFileTypes: true })
          .map((entry) => {
            if (entry.isSymbolicLink()) return `${entry.name}@`;
            if (entry.isDirectory()) return `${entry.name}/`;
            return entry.name;
          })
          .sort();
        const shown = names.slice(0, LIST_MAX_ENTRIES);
        if (names.length > LIST_MAX_ENTRIES) {
          shown.push(
            `[truncated: showing ${LIST_MAX_ENTRIES} of ${names.length} entries]`,
          );
        }
        return ok(shown.join("\n"));
      } catch (error) {
        return fail(errorText(error));
      }
    },
  };
}
